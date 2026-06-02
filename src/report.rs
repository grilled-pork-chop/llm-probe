//! Console renderer for a `Report` — the canonical human-readable report (§8)
//! the TUI also falls back to on exit, plus the stable `--json` shape (A.11).

use crate::config::RunConfig;
use crate::metrics::{ErrorKind, LatencyStats, Report};
use serde::Serialize;

/// Below this request count, percentiles are coarse; we say so (§8 / A.8).
const SMALL_N: usize = 20;

// ANSI SGR codes, applied only when color is enabled (A.12).
const ACCENT: &str = "1;36"; // bold cyan
const BOLD: &str = "1";
const GREEN: &str = "32";
const RED: &str = "31";

/// Wrap `s` in an SGR code when `on`, else return it plain.
fn paint(on: bool, code: &str, s: &str) -> String {
    if on {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Render the full §8 report to a `String` (stdout-bound by the caller).
/// `color` enables ANSI styling (decided by the caller per A.12).
pub fn render(report: &Report, cfg: &RunConfig, color: bool) -> String {
    let mut out = String::new();
    let mode = if cfg.stream {
        "streaming"
    } else {
        "non-streaming"
    };

    push_line(
        &mut out,
        format!(
            "{} — {}  (model: {})",
            paint(color, ACCENT, "llmprobe"),
            cfg.endpoint,
            cfg.model
        ),
    );
    let requests = if cfg.requests == 0 {
        "∞".to_string()
    } else {
        cfg.requests.to_string()
    };
    push_line(
        &mut out,
        format!(
            "mode: {mode} · requests: {requests} · concurrency: {} · max_tokens: {}",
            cfg.concurrency, cfg.max_tokens
        ),
    );
    out.push('\n');

    let ok_s = paint(color, GREEN, &format!("{} ok", report.ok));
    let failed_s = if report.failed > 0 {
        paint(color, RED, &format!("{} failed", report.failed))
    } else {
        format!("{} failed", report.failed)
    };
    push_line(
        &mut out,
        format!(
            "Requests    {} total   {ok_s}   {failed_s}   ({:.1}% success)",
            report.total,
            report.success_rate * 100.0
        ),
    );
    push_line(
        &mut out,
        format!(
            "Wall clock  {:.2} s     throughput {:.2} req/s",
            report.wall_clock.as_secs_f64(),
            report.req_per_s
        ),
    );

    if report.ok == 0 {
        out.push('\n');
        push_line(&mut out, "no successful requests".to_string());
    } else {
        render_latency(&mut out, report, color);
        render_throughput(&mut out, report, color);
    }

    render_errors(&mut out, report, color);

    if report.ok > 0 && report.total < SMALL_N {
        push_line(
            &mut out,
            format!(
                "(small N: percentiles over {} samples are approximate)",
                report.ok
            ),
        );
    }

    out
}

fn render_latency(out: &mut String, report: &Report, color: bool) {
    if let Some(e) = &report.e2e {
        push_line(out, paint(color, BOLD, "End-to-end latency"));
        push_line(
            out,
            format!(
                "  min {}   p50 {}   avg {}   p95 {}   p99 {}   max {}  (±{:.2})",
                secs(e.min),
                secs(e.p50),
                secs(e.avg),
                secs(e.p95),
                secs(e.p99),
                secs(e.max),
                e.stddev
            ),
        );
    }
    if let Some(t) = &report.ttft {
        push_line(out, paint(color, BOLD, "Time to first token"));
        push_line(
            out,
            format!(
                "  min {}   avg {}   p95 {}",
                secs(t.min),
                secs(t.avg),
                secs(t.p95)
            ),
        );
    }
}

fn render_throughput(out: &mut String, report: &Report, color: bool) {
    push_line(out, paint(color, BOLD, "Throughput"));
    push_line(
        out,
        format!(
            "  tokens/s   avg/req {}    aggregate {}    inter-token {}",
            opt_f1(report.avg_tps),
            opt_f1(report.aggregate_tps),
            itl(report.mean_itl_ms, report.max_gap_ms),
        ),
    );

    let len = match report.output_len {
        Some((mn, avg, mx)) => format!("min {mn}  avg {avg:.0}  max {mx}"),
        None => "—".to_string(),
    };
    push_line(
        out,
        format!(
            "  completion {} tok total   ·   output len  {len}",
            thousands(report.total_completion_tokens),
        ),
    );

    if let Some(sp) = report.speedup {
        push_line(out, format!("  concurrency speedup {sp:.1}×"));
    }
}

fn render_errors(out: &mut String, report: &Report, color: bool) {
    if report.error_breakdown.is_empty() {
        return;
    }
    push_line(out, paint(color, BOLD, "Errors"));
    for (kind, count) in &report.error_breakdown {
        push_line(out, format!("  {count}× {kind}"));
    }
}

fn push_line(out: &mut String, line: String) {
    out.push_str(&line);
    out.push('\n');
}

/// Format seconds with a unit, e.g. `1.62 s`.
fn secs(v: f64) -> String {
    format!("{v:.2} s")
}

/// One-decimal optional float, `—` when absent.
fn opt_f1(v: Option<f64>) -> String {
    v.map_or_else(|| "—".to_string(), |x| format!("{x:.1}"))
}

/// `12.4 ms (max gap 41 ms)`, or `—` when no inter-token data.
fn itl(mean_ms: Option<f64>, max_gap_ms: Option<f64>) -> String {
    match mean_ms {
        Some(m) => match max_gap_ms {
            Some(g) => format!("{m:.1} ms (max gap {g:.0} ms)"),
            None => format!("{m:.1} ms"),
        },
        None => "—".to_string(),
    }
}

// ---- JSON output (A.11) ---------------------------------------------------

/// The stable `--json` shape. Field order/names match A.11 exactly; optional
/// metrics serialize as `null` (no `skip_serializing_if`). Durations in seconds.
#[derive(Serialize)]
struct JsonReport<'a> {
    target: &'a str,
    model: &'a str,
    config: JsonConfig,
    summary: JsonSummary,
    latency_e2e_s: Option<LatencyStats>,
    ttft_s: Option<JsonTtft>,
    throughput: JsonThroughput,
    errors: Vec<JsonError>,
}

#[derive(Serialize)]
struct JsonConfig {
    requests: usize,
    concurrency: usize,
    stream: bool,
    max_tokens: u32,
}

#[derive(Serialize)]
struct JsonSummary {
    total: usize,
    ok: usize,
    failed: usize,
    success_rate: f64,
    wall_clock_s: f64,
    req_per_s: f64,
}

#[derive(Serialize)]
struct JsonTtft {
    min: f64,
    avg: f64,
    p95: f64,
}

#[derive(Serialize)]
struct JsonThroughput {
    tps_avg_per_req: Option<f64>,
    tps_aggregate: Option<f64>,
    mean_itl_ms: Option<f64>,
    completion_tokens_total: u64,
    speedup: Option<f64>,
}

#[derive(Serialize)]
struct JsonError {
    kind: String,
    count: usize,
}

/// Render the report as the A.11 JSON document (never colored).
pub fn render_json(report: &Report, cfg: &RunConfig) -> Result<String, serde_json::Error> {
    let doc = JsonReport {
        target: &cfg.endpoint,
        model: &cfg.model,
        config: JsonConfig {
            requests: cfg.requests,
            concurrency: cfg.concurrency,
            stream: cfg.stream,
            max_tokens: cfg.max_tokens,
        },
        summary: JsonSummary {
            total: report.total,
            ok: report.ok,
            failed: report.failed,
            success_rate: report.success_rate,
            wall_clock_s: report.wall_clock.as_secs_f64(),
            req_per_s: report.req_per_s,
        },
        latency_e2e_s: report.e2e,
        ttft_s: report.ttft.map(|t| JsonTtft {
            min: t.min,
            avg: t.avg,
            p95: t.p95,
        }),
        throughput: JsonThroughput {
            tps_avg_per_req: report.avg_tps,
            tps_aggregate: report.aggregate_tps,
            mean_itl_ms: report.mean_itl_ms,
            completion_tokens_total: report.total_completion_tokens,
            speedup: report.speedup,
        },
        errors: report
            .error_breakdown
            .iter()
            .map(|(kind, count)| JsonError {
                kind: error_label(*kind),
                count: *count,
            })
            .collect(),
    };
    serde_json::to_string_pretty(&doc)
}

/// Stable machine label for an error class.
fn error_label(kind: ErrorKind) -> String {
    match kind {
        ErrorKind::Http(code) => format!("http_{code}"),
        ErrorKind::Timeout => "timeout".to_string(),
        ErrorKind::Connect => "connect".to_string(),
        ErrorKind::Decode => "decode".to_string(),
        ErrorKind::Stream => "stream".to_string(),
    }
}

/// Group digits with thin separators, e.g. `2,560`.
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_groups() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(2560), "2,560");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }

    #[test]
    fn itl_handles_missing() {
        assert_eq!(itl(None, None), "—");
        assert_eq!(itl(Some(12.4), None), "12.4 ms");
        assert_eq!(itl(Some(12.4), Some(41.0)), "12.4 ms (max gap 41 ms)");
    }
}
