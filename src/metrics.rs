//! Measurement outcomes and (Phase 2+) aggregate statistics.
//!
//! A `RequestOutcome` is the per-request record the client produces; errors are
//! captured *into* it (never propagated). Aggregation turns a slice of these
//! into a `Report`.

use crate::error::ProbeError;
use serde::Serialize;
use std::fmt;
use std::time::Duration;

/// Classified failure cause, kept compact for error-rate breakdowns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Non-2xx HTTP status (the `u16` is the code; 429 is called out in reports).
    Http(u16),
    Timeout,
    Connect,
    Decode,
    Stream,
}

impl ErrorKind {
    /// Best-effort classification of a `ProbeError` into a breakdown bucket.
    pub fn from_probe(err: &ProbeError) -> Self {
        match err {
            ProbeError::Api { status, .. } => ErrorKind::Http(*status),
            ProbeError::Timeout => ErrorKind::Timeout,
            ProbeError::Decode(_) => ErrorKind::Decode,
            ProbeError::Stream(_) => ErrorKind::Stream,
            ProbeError::Http(e) => {
                if e.is_timeout() {
                    ErrorKind::Timeout
                } else if e.is_connect() {
                    ErrorKind::Connect
                } else if e.is_decode() {
                    ErrorKind::Decode
                } else {
                    ErrorKind::Connect
                }
            }
            ProbeError::Config(_) => ErrorKind::Connect,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Http(code) => write!(f, "HTTP {code}"),
            ErrorKind::Timeout => f.write_str("timeout"),
            ErrorKind::Connect => f.write_str("connect"),
            ErrorKind::Decode => f.write_str("decode"),
            ErrorKind::Stream => f.write_str("stream"),
        }
    }
}

/// Per-request measurement record (A.10). Optional fields are `None` when the
/// quantity is undefined for the request (see A.9), never `NaN`.
#[derive(Debug, Clone)]
pub struct RequestOutcome {
    pub id: usize,
    pub success: bool,
    /// End-to-end latency: send → response complete.
    pub e2e: Duration,
    /// Time to first content token (streaming only).
    pub ttft: Option<Duration>,
    /// Decode window: last_token − first_token (streaming only).
    pub gen_time: Option<Duration>,
    pub completion_tokens: Option<u32>,
    pub prompt_tokens: Option<u32>,
    /// Per-request decode throughput (tokens/s).
    pub tps: Option<f64>,
    /// Mean inter-token latency, milliseconds.
    pub itl_ms: Option<f64>,
    /// Largest gap between consecutive tokens, milliseconds.
    pub max_gap_ms: Option<f64>,
    /// `None` on success.
    pub error: Option<ErrorKind>,
    /// Captured payload for inspection: the assistant reply on success, or the
    /// (truncated) error response body on failure. `None` when empty/unavailable.
    pub body: Option<String>,
}

impl RequestOutcome {
    /// A failed outcome carrying its id, elapsed time, classified error, and an
    /// optional captured error body.
    pub fn failed(id: usize, e2e: Duration, error: ErrorKind, body: Option<String>) -> Self {
        Self {
            id,
            success: false,
            e2e,
            ttft: None,
            gen_time: None,
            completion_tokens: None,
            prompt_tokens: None,
            tps: None,
            itl_ms: None,
            max_gap_ms: None,
            error: Some(error),
            body,
        }
    }
}

/// Latency distribution over the e2e (or TTFT) samples of successful requests.
/// All values in **seconds**. Field names match the A.11 `latency_e2e_s` object.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LatencyStats {
    pub min: f64,
    pub p50: f64,
    pub avg: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub stddev: f64,
}

impl LatencyStats {
    /// Compute stats from an unsorted sample set; `None` when empty.
    pub fn from_samples(samples: &[f64]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let n = sorted.len() as f64;
        let sum: f64 = sorted.iter().sum();
        let avg = sum / n;
        let variance = sorted.iter().map(|v| (v - avg).powi(2)).sum::<f64>() / n;
        Some(Self {
            min: sorted[0],
            p50: percentile(&sorted, 50.0).unwrap_or(avg),
            avg,
            p95: percentile(&sorted, 95.0).unwrap_or(avg),
            p99: percentile(&sorted, 99.0).unwrap_or(avg),
            max: sorted[sorted.len() - 1],
            stddev: variance.sqrt(),
        })
    }
}

/// Aggregate report over a whole batch (A.10). Optional metrics are `None` when
/// undefined (no successes, no streaming, missing usage — see A.9).
#[derive(Debug, Clone)]
pub struct Report {
    pub total: usize,
    pub ok: usize,
    pub failed: usize,
    pub success_rate: f64,
    pub wall_clock: Duration,
    pub req_per_s: f64,
    pub e2e: Option<LatencyStats>,
    pub ttft: Option<LatencyStats>,
    /// Mean of per-request decode TPS over successes that have it.
    pub avg_tps: Option<f64>,
    /// Σ completion_tokens / Σ generation_time across successes.
    pub aggregate_tps: Option<f64>,
    pub mean_itl_ms: Option<f64>,
    pub max_gap_ms: Option<f64>,
    pub total_completion_tokens: u64,
    pub total_prompt_tokens: u64,
    /// (min, avg, max) completion tokens over successful requests.
    pub output_len: Option<(u32, f64, u32)>,
    /// aggregate_tps ÷ avg_tps (≈1 means no parallel scaling).
    pub speedup: Option<f64>,
    /// Failure counts by class, highest first (429 surfaces as its own bucket).
    pub error_breakdown: Vec<(ErrorKind, usize)>,
}

/// Nearest-rank percentile (A.8). `sorted` ascending, in seconds; `p` in 0..=100.
pub fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize; // 1-based
    let idx = rank.clamp(1, sorted.len()) - 1;
    Some(sorted[idx])
}

/// Reduce per-request outcomes into a `Report` (synchronous; §7).
pub fn aggregate(outcomes: &[RequestOutcome], wall_clock: Duration) -> Report {
    let total = outcomes.len();
    let successes: Vec<&RequestOutcome> = outcomes.iter().filter(|o| o.success).collect();
    let ok = successes.len();
    let failed = total - ok;
    let success_rate = if total == 0 {
        0.0
    } else {
        ok as f64 / total as f64
    };
    let wall_secs = wall_clock.as_secs_f64();
    let req_per_s = if wall_secs > 0.0 {
        ok as f64 / wall_secs
    } else {
        0.0
    };

    let e2e_samples: Vec<f64> = successes.iter().map(|o| o.e2e.as_secs_f64()).collect();
    let e2e = LatencyStats::from_samples(&e2e_samples);

    let ttft_samples: Vec<f64> = successes
        .iter()
        .filter_map(|o| o.ttft.map(|d| d.as_secs_f64()))
        .collect();
    let ttft = LatencyStats::from_samples(&ttft_samples);

    let tps_samples: Vec<f64> = successes.iter().filter_map(|o| o.tps).collect();
    let avg_tps = mean(&tps_samples);

    let total_completion_tokens: u64 = successes
        .iter()
        .filter_map(|o| o.completion_tokens)
        .map(u64::from)
        .sum();
    let total_prompt_tokens: u64 = successes
        .iter()
        .filter_map(|o| o.prompt_tokens)
        .map(u64::from)
        .sum();

    // Aggregate (deployment) TPS: total completion tokens over the wall-clock
    // window. Scales with concurrency, unlike per-request decode rate — which is
    // what makes `speedup = aggregate / avg` meaningful (§2.1).
    let aggregate_tps = if wall_secs > 0.0 && total_completion_tokens > 0 {
        Some(total_completion_tokens as f64 / wall_secs)
    } else {
        None
    };

    let speedup = match (aggregate_tps, avg_tps) {
        (Some(agg), Some(per)) if per > 0.0 => Some(agg / per),
        _ => None,
    };

    let itl_samples: Vec<f64> = successes.iter().filter_map(|o| o.itl_ms).collect();
    let mean_itl_ms = mean(&itl_samples);
    let max_gap_ms = successes
        .iter()
        .filter_map(|o| o.max_gap_ms)
        .fold(None, |acc: Option<f64>, v| {
            Some(acc.map_or(v, |m| m.max(v)))
        });

    let lens: Vec<u32> = successes
        .iter()
        .filter_map(|o| o.completion_tokens)
        .collect();
    let output_len = if lens.is_empty() {
        None
    } else {
        let min = *lens.iter().min().unwrap_or(&0);
        let max = *lens.iter().max().unwrap_or(&0);
        let avg = lens.iter().map(|&v| f64::from(v)).sum::<f64>() / lens.len() as f64;
        Some((min, avg, max))
    };

    let error_breakdown = error_breakdown(outcomes);

    Report {
        total,
        ok,
        failed,
        success_rate,
        wall_clock,
        req_per_s,
        e2e,
        ttft,
        avg_tps,
        aggregate_tps,
        mean_itl_ms,
        max_gap_ms,
        total_completion_tokens,
        total_prompt_tokens,
        output_len,
        speedup,
        error_breakdown,
    }
}

/// Count failures by `ErrorKind`, highest count first (stable within ties).
fn error_breakdown(outcomes: &[RequestOutcome]) -> Vec<(ErrorKind, usize)> {
    let mut counts: Vec<(ErrorKind, usize)> = Vec::new();
    for o in outcomes.iter().filter(|o| !o.success) {
        if let Some(kind) = o.error {
            match counts.iter_mut().find(|(k, _)| *k == kind) {
                Some((_, c)) => *c += 1,
                None => counts.push((kind, 1)),
            }
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    counts
}

/// Mean of a sample set, or `None` when empty.
fn mean(samples: &[f64]) -> Option<f64> {
    if samples.is_empty() {
        None
    } else {
        Some(samples.iter().sum::<f64>() / samples.len() as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_empty_is_none() {
        assert_eq!(percentile(&[], 50.0), None);
    }

    #[test]
    fn percentile_single_value() {
        let s = [1.5];
        for p in [0.0, 50.0, 95.0, 99.0, 100.0] {
            assert_eq!(percentile(&s, p), Some(1.5));
        }
    }

    #[test]
    fn percentile_nearest_rank() {
        let s = [1.0, 2.0, 3.0, 4.0, 5.0];
        // ceil(p/100 * 5): p50 -> rank 3 -> 3.0; p95 -> rank 5 -> 5.0
        assert_eq!(percentile(&s, 50.0), Some(3.0));
        assert_eq!(percentile(&s, 95.0), Some(5.0));
        assert_eq!(percentile(&s, 99.0), Some(5.0));
        assert_eq!(percentile(&s, 100.0), Some(5.0));
        // p in (0,20] -> rank 1 -> 1.0
        assert_eq!(percentile(&s, 1.0), Some(1.0));
        assert_eq!(percentile(&s, 20.0), Some(1.0));
    }

    #[test]
    fn percentile_with_duplicates() {
        let s = [2.0, 2.0, 2.0, 2.0];
        assert_eq!(percentile(&s, 50.0), Some(2.0));
        assert_eq!(percentile(&s, 95.0), Some(2.0));
    }

    #[test]
    fn latency_stats_basic() {
        let st = LatencyStats::from_samples(&[5.0, 1.0, 3.0, 2.0, 4.0]).unwrap();
        assert_eq!(st.min, 1.0);
        assert_eq!(st.max, 5.0);
        assert_eq!(st.avg, 3.0);
        assert_eq!(st.p50, 3.0);
        assert!((st.stddev - std::f64::consts::SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn latency_stats_empty_is_none() {
        assert!(LatencyStats::from_samples(&[]).is_none());
    }
}
