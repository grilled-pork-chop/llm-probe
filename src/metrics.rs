//! Per-turn and per-conversation measurement records, plus aggregate statistics.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

// ── Error classification ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    ContextOverflow,
    Http(u16),
    Timeout,
    Connect,
    Decode,
    Stream,
    /// Invalid configuration detected before any HTTP request was made.
    Config,
}

impl ErrorKind {
    pub fn from_probe(err: &crate::error::ProbeError) -> Self {
        use crate::error::ProbeError::*;
        match err {
            ContextOverflow { .. } => ErrorKind::ContextOverflow,
            Api { status, .. } => ErrorKind::Http(*status),
            Timeout => ErrorKind::Timeout,
            Decode(_) => ErrorKind::Decode,
            Stream(_) => ErrorKind::Stream,
            Http(e) => {
                if e.is_timeout() {
                    ErrorKind::Timeout
                } else {
                    ErrorKind::Connect
                }
            }
            Config(_) => ErrorKind::Config,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::ContextOverflow => f.write_str("ctx-limit"),
            ErrorKind::Http(c) => write!(f, "HTTP {c}"),
            ErrorKind::Timeout => f.write_str("timeout"),
            ErrorKind::Connect => f.write_str("connect"),
            ErrorKind::Decode => f.write_str("decode"),
            ErrorKind::Stream => f.write_str("stream"),
            ErrorKind::Config => f.write_str("config"),
        }
    }
}

// ── Per-turn outcome ──────────────────────────────────────────────────────────

/// Measurement record for one turn inside a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnOutcome {
    pub conv_id: usize,
    pub turn_idx: usize,
    /// Prompt (input) tokens sent to the model this turn.
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    /// End-to-end latency: send → last byte received.
    pub e2e: Duration,
    /// Time to first content token (streaming only).
    pub ttft: Option<Duration>,
    /// `(e2e − ttft) / (completion_tokens − 1)` in ms; the per-step decode latency.
    pub tpot_ms: Option<f64>,
    /// Mean of observed inter-token wall-clock gaps (streaming only).
    pub itl_ms: Option<f64>,
    /// `completion_tokens / gen_time` in tokens/s.
    pub tps: Option<f64>,
    pub success: bool,
    pub error: Option<ErrorKind>,
    /// The user prompt sent this turn (pool seed on turn 0, follow-up after).
    /// Stored so the report can show real text rather than placeholders.
    /// `#[serde(default)]` tolerates JSON records without this field.
    #[serde(default)]
    pub prompt: String,
    /// Captured assistant reply text, used to grow conversation history.
    pub reply: Option<String>,
}

impl TurnOutcome {
    pub fn failed(conv_id: usize, turn_idx: usize, e2e: Duration, error: ErrorKind) -> Self {
        Self {
            conv_id,
            turn_idx,
            prompt_tokens: None,
            completion_tokens: None,
            e2e,
            ttft: None,
            tpot_ms: None,
            itl_ms: None,
            tps: None,
            success: false,
            error: Some(error),
            prompt: String::new(),
            reply: None,
        }
    }
}

// ── Per-conversation outcome ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalReason {
    ContextLimit,
    MaxTurns,
    Error(ErrorKind),
    Cancelled,
}

impl fmt::Display for TerminalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TerminalReason::ContextLimit => f.write_str("ctx-limit"),
            TerminalReason::MaxTurns => f.write_str("max-turns"),
            TerminalReason::Error(k) => write!(f, "error({k})"),
            TerminalReason::Cancelled => f.write_str("cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationOutcome {
    pub id: usize,
    pub slot: usize,
    /// System prompt used for this conversation (override or pool pick).
    /// `#[serde(default)]` tolerates JSON records without this field.
    #[serde(default)]
    pub system: String,
    pub turns: Vec<TurnOutcome>,
    pub terminal: TerminalReason,
    pub wall_clock: Duration,
}

impl ConversationOutcome {
    /// Successful turns (excludes the final overflow/error turn).
    pub fn ok_turns(&self) -> impl Iterator<Item = &TurnOutcome> {
        self.turns.iter().filter(|t| t.success)
    }

    /// Total prompt tokens of the last successful turn (context depth reached).
    pub fn context_depth(&self) -> Option<u32> {
        self.turns
            .iter()
            .rev()
            .find_map(|t| if t.success { t.prompt_tokens } else { None })
    }

    pub fn avg_ttft_ms(&self) -> Option<f64> {
        mean(
            &self
                .ok_turns()
                .filter_map(|t| t.ttft.map(|d| d.as_secs_f64() * 1000.0))
                .collect::<Vec<_>>(),
        )
    }

    pub fn avg_tps(&self) -> Option<f64> {
        mean(&self.ok_turns().filter_map(|t| t.tps).collect::<Vec<_>>())
    }

    pub fn total_prompt_tokens(&self) -> u64 {
        self.ok_turns()
            .filter_map(|t| t.prompt_tokens)
            .map(u64::from)
            .sum()
    }

    pub fn total_completion_tokens(&self) -> u64 {
        self.ok_turns()
            .filter_map(|t| t.completion_tokens)
            .map(u64::from)
            .sum()
    }
}

/// Full message list sent to the API at `turn_idx`, reconstructed by interleaving
/// each turn's stored prompt with its captured reply. Mirrors the growth loop in
/// `runner::run_conversation`. Takes a `turns` slice so both completed and
/// in-flight (partial) conversations can be reconstructed.
pub fn request_messages(turns: &[TurnOutcome], turn_idx: usize) -> Vec<(String, String)> {
    let mut msgs = Vec::new();
    for t in turns.iter().take(turn_idx) {
        msgs.push(("user".into(), t.prompt.clone()));
        msgs.push(("assistant".into(), t.reply.clone().unwrap_or_default()));
    }
    if let Some(t) = turns.get(turn_idx) {
        msgs.push(("user".into(), t.prompt.clone()));
    }
    msgs
}

// ── Run-level result ──────────────────────────────────────────────────────────

/// Snapshot of `RunConfig` fields needed for report display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub endpoint: String,
    pub model: String,
    pub concurrency: usize,
    pub stream: bool,
    pub max_tokens: Option<u32>,
}

/// The complete output of a grow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrowResult {
    pub conversations: Vec<ConversationOutcome>,
    pub wall_clock: Duration,
    pub config: ConfigSnapshot,
}

// ── Aggregate report ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TurnStats {
    pub ttft: Option<LatencyStats>,
    /// TPOT in ms.
    pub tpot: Option<LatencyStats>,
    /// ITL in ms.
    pub itl: Option<LatencyStats>,
    /// e2e in seconds.
    pub e2e: Option<LatencyStats>,
    /// Per-request TPS.
    pub tps: Option<LatencyStats>,
    /// Aggregate TPS: total completion tokens / wall_clock.
    pub aggregate_tps: Option<f64>,
    pub total_turns: usize,
    pub ok_turns: usize,
    pub total_completion_tokens: u64,
    pub output_len: Option<(u32, f64, u32)>,
    pub success_rate: f64,
}

#[derive(Debug, Clone)]
pub struct ConvStats {
    pub total: usize,
    pub context_limit: usize,
    pub max_turns_hit: usize,
    pub errors: usize,
    pub cancelled: usize,
    /// Distribution of turns-to-limit.
    pub turns_dist: Option<LatencyStats>,
    /// Distribution of context depth at limit (tokens).
    pub context_depth_dist: Option<LatencyStats>,
}

#[derive(Debug, Clone)]
pub struct DegradationBucket {
    pub label: String,
    pub avg_tpot_ms: f64,
    pub pct_change: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct RunReport {
    pub wall_clock: Duration,
    pub concurrency: usize,
    pub turns: TurnStats,
    pub conversations: ConvStats,
    /// TPOT degradation bucketed by turn index.
    pub degradation: Vec<DegradationBucket>,
}

/// Build the aggregate report from a completed or partial `GrowResult`.
pub fn aggregate(result: &GrowResult) -> RunReport {
    let wall_secs = result.wall_clock.as_secs_f64();
    let all_turns: Vec<&TurnOutcome> = result
        .conversations
        .iter()
        .flat_map(|c| c.turns.iter())
        .collect();
    let ok_turns: Vec<&TurnOutcome> = all_turns.iter().filter(|t| t.success).copied().collect();

    // A context-overflow turn is the designed terminal of grow mode, not a
    // failure, so it is excluded from the success-rate denominator. Real errors
    // (HTTP, timeout, connect, decode, stream) still count.
    let total_turns = all_turns
        .iter()
        .filter(|t| t.error != Some(ErrorKind::ContextOverflow))
        .count();
    let ok_turn_count = ok_turns.len();
    let success_rate = if total_turns == 0 {
        1.0
    } else {
        ok_turn_count as f64 / total_turns as f64
    };

    let total_completion_tokens: u64 = ok_turns
        .iter()
        .filter_map(|t| t.completion_tokens)
        .map(u64::from)
        .sum();

    let aggregate_tps = if wall_secs > 0.0 && total_completion_tokens > 0 {
        Some(total_completion_tokens as f64 / wall_secs)
    } else {
        None
    };

    let ttft_ms: Vec<f64> = ok_turns
        .iter()
        .filter_map(|t| t.ttft.map(|d| d.as_secs_f64() * 1000.0))
        .collect();
    let tpot_samples: Vec<f64> = ok_turns.iter().filter_map(|t| t.tpot_ms).collect();
    let itl_samples: Vec<f64> = ok_turns.iter().filter_map(|t| t.itl_ms).collect();
    let e2e_samples: Vec<f64> = ok_turns.iter().map(|t| t.e2e.as_secs_f64()).collect();
    let tps_samples: Vec<f64> = ok_turns.iter().filter_map(|t| t.tps).collect();

    let lens: Vec<u32> = ok_turns
        .iter()
        .filter_map(|t| t.completion_tokens)
        .collect();
    let output_len = if lens.is_empty() {
        None
    } else {
        let mn = *lens.iter().min().unwrap_or(&0);
        let mx = *lens.iter().max().unwrap_or(&0);
        let avg = lens.iter().map(|&v| f64::from(v)).sum::<f64>() / lens.len() as f64;
        Some((mn, avg, mx))
    };

    let turns = TurnStats {
        ttft: LatencyStats::from_samples(&ttft_ms),
        tpot: LatencyStats::from_samples(&tpot_samples),
        itl: LatencyStats::from_samples(&itl_samples),
        e2e: LatencyStats::from_samples(&e2e_samples),
        tps: LatencyStats::from_samples(&tps_samples),
        aggregate_tps,
        total_turns,
        ok_turns: ok_turn_count,
        total_completion_tokens,
        output_len,
        success_rate,
    };

    let conv_total = result.conversations.len();
    let mut ctx_limit = 0;
    let mut max_turns_hit = 0;
    let mut errors = 0;
    let mut cancelled = 0;
    let mut turns_counts: Vec<f64> = Vec::new();
    let mut depths: Vec<f64> = Vec::new();

    for conv in &result.conversations {
        match conv.terminal {
            TerminalReason::ContextLimit => {
                ctx_limit += 1;
                turns_counts.push(conv.ok_turns().count() as f64);
                if let Some(d) = conv.context_depth() {
                    depths.push(f64::from(d));
                }
            }
            TerminalReason::MaxTurns => max_turns_hit += 1,
            TerminalReason::Error(_) => errors += 1,
            TerminalReason::Cancelled => cancelled += 1,
        }
    }

    let conversations = ConvStats {
        total: conv_total,
        context_limit: ctx_limit,
        max_turns_hit,
        errors,
        cancelled,
        turns_dist: LatencyStats::from_samples(&turns_counts),
        context_depth_dist: LatencyStats::from_samples(&depths),
    };

    let degradation = build_degradation(&result.conversations);

    RunReport {
        wall_clock: result.wall_clock,
        concurrency: result.config.concurrency,
        turns,
        conversations,
        degradation,
    }
}

/// Bucket turns by index (1-4, 5-8, …) and compute average TPOT per bucket.
fn build_degradation(convs: &[ConversationOutcome]) -> Vec<DegradationBucket> {
    const BUCKET: usize = 4;
    // Gather (turn_idx, tpot_ms) for all ok turns.
    let mut max_idx: usize = 0;
    for conv in convs {
        if let Some(t) = conv.ok_turns().last() {
            max_idx = max_idx.max(t.turn_idx);
        }
    }
    if max_idx == 0 {
        return vec![];
    }

    let num_buckets = (max_idx / BUCKET) + 1;
    let mut sums = vec![0.0_f64; num_buckets];
    let mut counts = vec![0usize; num_buckets];

    for conv in convs {
        for t in conv.ok_turns() {
            if let Some(tpot) = t.tpot_ms {
                let b = t.turn_idx / BUCKET;
                if b < num_buckets {
                    sums[b] += tpot;
                    counts[b] += 1;
                }
            }
        }
    }

    let mut result: Vec<DegradationBucket> = Vec::new();
    let mut first_avg: Option<f64> = None;
    for (i, (&s, &c)) in sums.iter().zip(counts.iter()).enumerate() {
        if c == 0 {
            continue;
        }
        let avg = s / c as f64;
        let lo = i * BUCKET + 1;
        let hi = lo + BUCKET - 1;
        let label = format!("turns {lo}–{hi}");
        let pct_change = first_avg.map(|base| (avg - base) / base * 100.0);
        if first_avg.is_none() {
            first_avg = Some(avg);
        }
        result.push(DegradationBucket {
            label,
            avg_tpot_ms: avg,
            pct_change,
        });
    }
    result
}

// ── LatencyStats ─────────────────────────────────────────────────────────────

/// Latency distribution. Field values are in whatever unit the caller uses.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

pub fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.clamp(1, sorted.len()) - 1;
    Some(sorted[idx])
}

pub fn mean(samples: &[f64]) -> Option<f64> {
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
    fn percentile_nearest_rank() {
        let s = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&s, 50.0), Some(3.0));
        assert_eq!(percentile(&s, 95.0), Some(5.0));
    }

    #[test]
    fn latency_stats_basic() {
        let st = LatencyStats::from_samples(&[5.0, 1.0, 3.0, 2.0, 4.0]).unwrap();
        assert_eq!(st.min, 1.0);
        assert_eq!(st.max, 5.0);
        assert_eq!(st.avg, 3.0);
    }

    #[test]
    fn degradation_buckets() {
        // Two conversations, each with 8 turns with growing TPOT.
        let make_turn = |conv_id: usize, idx: usize, tpot: f64| TurnOutcome {
            conv_id,
            turn_idx: idx,
            prompt_tokens: Some(100 * (idx as u32 + 1)),
            completion_tokens: Some(50),
            e2e: Duration::from_millis(500),
            ttft: Some(Duration::from_millis(200)),
            tpot_ms: Some(tpot),
            itl_ms: None,
            tps: Some(80.0),
            success: true,
            error: None,
            prompt: "prompt".into(),
            reply: Some("text".into()),
        };
        let conv = ConversationOutcome {
            id: 0,
            slot: 0,
            system: "system".into(),
            turns: (0..8).map(|i| make_turn(0, i, 10.0 + i as f64)).collect(),
            terminal: TerminalReason::ContextLimit,
            wall_clock: Duration::from_secs(10),
        };
        let result = GrowResult {
            conversations: vec![conv],
            wall_clock: Duration::from_secs(10),
            config: ConfigSnapshot {
                endpoint: "http://x".into(),
                model: "m".into(),
                concurrency: 1,
                stream: true,
                max_tokens: None,
            },
        };
        let report = aggregate(&result);
        assert!(report.degradation.len() >= 2, "expected at least 2 buckets");
        assert!(
            report.degradation[1].pct_change.unwrap() > 0.0,
            "TPOT should grow"
        );
    }

    #[test]
    fn request_messages_reconstructs_history() {
        let turn = |idx: usize, prompt: &str, reply: &str| TurnOutcome {
            conv_id: 0,
            turn_idx: idx,
            prompt_tokens: Some(10),
            completion_tokens: Some(5),
            e2e: Duration::from_millis(100),
            ttft: None,
            tpot_ms: None,
            itl_ms: None,
            tps: None,
            success: true,
            error: None,
            prompt: prompt.into(),
            reply: Some(reply.into()),
        };
        let conv = ConversationOutcome {
            id: 0,
            slot: 0,
            system: "system".into(),
            turns: vec![
                turn(0, "seed-prompt", "reply-0"),
                turn(1, "followup-prompt", "reply-1"),
            ],
            terminal: TerminalReason::ContextLimit,
            wall_clock: Duration::from_secs(1),
        };
        // Turn 0 sees only its own (seed) prompt.
        assert_eq!(
            request_messages(&conv.turns, 0),
            vec![("user".into(), "seed-prompt".into())]
        );

        // Turn 1 sees the seed, its reply, and this turn's follow-up prompt.
        assert_eq!(
            request_messages(&conv.turns, 1),
            vec![
                ("user".into(), "seed-prompt".into()),
                ("assistant".into(), "reply-0".into()),
                ("user".into(), "followup-prompt".into()),
            ]
        );
    }
}
