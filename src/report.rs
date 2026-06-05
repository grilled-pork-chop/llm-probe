//! Console and JSON report renderer for a completed grow run.

use crate::fmt::thousands;
use crate::metrics::{
    ConvStats, DegradationBucket, GrowResult, LatencyStats, TurnStats, aggregate,
};
use serde::Serialize;

const SMALL_N: usize = 20;
const ACCENT: &str = "1;36";
const BOLD: &str = "1";
const GREEN: &str = "32";
const RED: &str = "31";
const YELLOW: &str = "33";

fn paint(on: bool, code: &str, s: &str) -> String {
    if on {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

// ── Plain-text report ─────────────────────────────────────────────────────────

pub fn render(result: &GrowResult, color: bool) -> String {
    let report = aggregate(result);
    let cfg = &result.config;
    let mut out = String::new();

    let mode = if cfg.stream { "streaming" } else { "non-streaming" };

    push(&mut out, format!(
        "{} — {}  (model: {})",
        paint(color, ACCENT, "llmprobe"),
        cfg.endpoint,
        cfg.model
    ));
    push(&mut out, format!(
        "mode: {mode} · slots: {} · max-tokens: {} · turn: \"{}\"",
        cfg.concurrency,
        cfg.max_tokens.map_or("unlimited".into(), |n| n.to_string()),
        cfg.turn_prompt
    ));
    out.push('\n');

    render_conversations(&mut out, &report.conversations, color);
    out.push('\n');

    if report.turns.ok_turns > 0 {
        render_ttft(&mut out, &report.turns, color);
        render_tpot(&mut out, &report.turns, color);
        render_tps(&mut out, &report.turns, color);
        render_throughput(&mut out, &report.turns, color);
        out.push('\n');
        render_degradation(&mut out, &report.degradation, color);
    }

    if report.turns.ok_turns > 0 && report.turns.ok_turns < SMALL_N {
        push(&mut out, format!(
            "(small N: percentiles over {} turns are approximate)",
            report.turns.ok_turns
        ));
    }

    out
}

fn render_conversations(out: &mut String, cs: &ConvStats, color: bool) {
    push(out, paint(color, BOLD, "Conversations"));
    let ctx_s = paint(color, GREEN, &format!("{} ctx-limit", cs.context_limit));
    let err_s = if cs.errors > 0 {
        paint(color, RED, &format!("{} error", cs.errors))
    } else {
        format!("{} error", cs.errors)
    };
    push(out, format!(
        "  {} total   {ctx_s}   {err_s}   {} max-turns   {} cancelled",
        cs.total, cs.max_turns_hit, cs.cancelled
    ));

    if let Some(d) = &cs.turns_dist {
        push(out, format!(
            "  turns to limit   min {:.0}   p50 {:.0}   p95 {:.0}   p99 {:.0}   max {:.0}",
            d.min, d.p50, d.p95, d.p99, d.max
        ));
    }
    if let Some(d) = &cs.context_depth_dist {
        push(out, format!(
            "  tokens at limit  min {}   p50 {}   p95 {}   max {}",
            d.min as u64, d.p50 as u64, d.p95 as u64, d.max as u64
        ));
    }
}

fn render_ttft(out: &mut String, ts: &TurnStats, color: bool) {
    if let Some(s) = &ts.ttft {
        push(out, paint(color, BOLD, "Time to first token (ms)"));
        push(out, format!(
            "  p50 {:.0}   p95 {:.0}   p99 {:.0}   avg {:.0}   min {:.0}   max {:.0}",
            s.p50, s.p95, s.p99, s.avg, s.min, s.max
        ));
    }
}

fn render_tpot(out: &mut String, ts: &TurnStats, color: bool) {
    if let Some(s) = &ts.tpot {
        push(out, paint(color, BOLD, "TPOT / decode latency (ms)"));
        push(out, format!(
            "  p50 {:.1}   p95 {:.1}   p99 {:.1}   avg {:.1}   min {:.1}   max {:.1}",
            s.p50, s.p95, s.p99, s.avg, s.min, s.max
        ));
    }
    if let Some(s) = &ts.itl {
        push(out, format!(
            "  ITL (inter-token)   avg {:.1} ms   p95 {:.1} ms",
            s.avg, s.p95
        ));
    }
}

fn render_tps(out: &mut String, ts: &TurnStats, color: bool) {
    if ts.tps.is_none() && ts.aggregate_tps.is_none() {
        return;
    }
    push(out, paint(color, BOLD, "TPS"));
    if let Some(s) = &ts.tps {
        push(out, format!(
            "  per-req   p50 {:.1}   p95 {:.1}   p99 {:.1}   avg {:.1}",
            s.p50, s.p95, s.p99, s.avg
        ));
    }
    if let Some(a) = ts.aggregate_tps {
        push(out, format!("  aggregate {a:.1} tok/s"));
    }
}

fn render_throughput(out: &mut String, ts: &TurnStats, color: bool) {
    push(out, paint(color, BOLD, "Throughput"));
    let ok_pct = ts.success_rate * 100.0;
    push(out, format!(
        "  success rate {ok_pct:.1}%   turns ok {} / {}",
        ts.ok_turns, ts.total_turns
    ));
    push(out, format!(
        "  completion {} tok total",
        thousands(ts.total_completion_tokens)
    ));
    if let Some((mn, avg, mx)) = ts.output_len {
        push(out, format!("  output len   min {mn}   avg {avg:.0}   max {mx}"));
    }
    if let Some(s) = &ts.e2e {
        push(out, format!(
            "  e2e latency (s)   p50 {:.2}   p95 {:.2}   avg {:.2}",
            s.p50, s.p95, s.avg
        ));
    }
}

fn render_degradation(out: &mut String, buckets: &[DegradationBucket], color: bool) {
    if buckets.is_empty() {
        return;
    }
    push(out, paint(color, BOLD, "TPOT degradation by turn index (context growth effect)"));
    for b in buckets {
        let change = match b.pct_change {
            Some(p) if p >= 0.0 => paint(color, YELLOW, &format!("+{p:.0}%")),
            Some(p) => format!("{p:.0}%"),
            None => String::new(),
        };
        push(out, format!("  {:15}  {:6.1} ms  {}", b.label, b.avg_tpot_ms, change));
    }
}

fn push(out: &mut String, line: String) {
    out.push_str(&line);
    out.push('\n');
}

// ── JSON output ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct JsonReport<'a> {
    target: &'a str,
    model: &'a str,
    config: JsonConfig<'a>,
    conversations: JsonConvStats,
    turns: JsonTurnStats,
    degradation: Vec<JsonDegBucket>,
}

#[derive(Serialize)]
struct JsonConfig<'a> {
    concurrency: usize,
    stream: bool,
    max_tokens: Option<u32>,
    turn_prompt: &'a str,
}

#[derive(Serialize)]
struct JsonConvStats {
    total: usize,
    context_limit: usize,
    max_turns_hit: usize,
    errors: usize,
    turns_to_limit: Option<LatencyStats>,
    tokens_at_limit: Option<LatencyStats>,
}

#[derive(Serialize)]
struct JsonTurnStats {
    total: usize,
    ok: usize,
    success_rate: f64,
    ttft_ms: Option<LatencyStats>,
    tpot_ms: Option<LatencyStats>,
    itl_ms: Option<LatencyStats>,
    e2e_s: Option<LatencyStats>,
    tps: Option<LatencyStats>,
    aggregate_tps: Option<f64>,
    completion_tokens_total: u64,
}

#[derive(Serialize)]
struct JsonDegBucket {
    label: String,
    avg_tpot_ms: f64,
    pct_change: Option<f64>,
}

pub fn render_json(result: &GrowResult) -> Result<String, serde_json::Error> {
    let report = aggregate(result);
    let cfg = &result.config;

    let doc = JsonReport {
        target: &cfg.endpoint,
        model: &cfg.model,
        config: JsonConfig {
            concurrency: cfg.concurrency,
            stream: cfg.stream,
            max_tokens: cfg.max_tokens,
            turn_prompt: &cfg.turn_prompt,
        },
        conversations: JsonConvStats {
            total: report.conversations.total,
            context_limit: report.conversations.context_limit,
            max_turns_hit: report.conversations.max_turns_hit,
            errors: report.conversations.errors,
            turns_to_limit: report.conversations.turns_dist,
            tokens_at_limit: report.conversations.context_depth_dist,
        },
        turns: JsonTurnStats {
            total: report.turns.total_turns,
            ok: report.turns.ok_turns,
            success_rate: report.turns.success_rate,
            ttft_ms: report.turns.ttft,
            tpot_ms: report.turns.tpot,
            itl_ms: report.turns.itl,
            e2e_s: report.turns.e2e,
            tps: report.turns.tps,
            aggregate_tps: report.turns.aggregate_tps,
            completion_tokens_total: report.turns.total_completion_tokens,
        },
        degradation: report
            .degradation
            .into_iter()
            .map(|b| JsonDegBucket {
                label: b.label,
                avg_tpot_ms: b.avg_tpot_ms,
                pct_change: b.pct_change,
            })
            .collect(),
    };
    serde_json::to_string_pretty(&doc)
}

