//! Command-line surface (§10 / A.3).
//!
//! The full flag set is declared up front so scripts don't break across builds
//! (e.g. `--tui` always parses; A.13). Flags are wired up over phases 1–6.

use clap::Parser;

/// Default prompt (A.3): a short, fixed user message.
pub const DEFAULT_PROMPT: &str = "Write three sentences about the history of computing.";

#[derive(Debug, Parser)]
#[command(
    name = "llmprobe",
    about = "Smoke-test an OpenAI-compatible chat endpoint",
    version
)]
pub struct Args {
    /// Base or full endpoint. If it doesn't end in /chat/completions, that path is appended.
    #[arg(short, long)]
    pub url: String,

    /// Model name.
    #[arg(short, long)]
    pub model: String,

    /// Number of requests; 0 (the default) runs indefinitely until interrupted.
    #[arg(short = 'n', long, default_value_t = 0)]
    pub requests: usize,

    /// Max in-flight requests.
    #[arg(short, long, default_value_t = 1)]
    pub concurrency: usize,

    /// Enable streaming + TTFT measurement.
    #[arg(long)]
    pub stream: bool,

    /// Prompt text.
    #[arg(short, long, default_value = DEFAULT_PROMPT)]
    pub prompt: String,

    /// Cap output tokens.
    #[arg(long, default_value_t = 128)]
    pub max_tokens: u32,

    /// Sampling temperature (omitted from the request when unset).
    #[arg(long)]
    pub temperature: Option<f32>,

    /// Per-request timeout in seconds.
    #[arg(long, default_value_t = 30)]
    pub timeout: u64,

    /// Discard the first K requests to exclude cold-start skew.
    #[arg(long, default_value_t = 0)]
    pub warmup: usize,

    /// Bearer token (falls back to $OPENAI_API_KEY).
    #[arg(long, env = "OPENAI_API_KEY")]
    pub api_key: Option<String>,

    /// Extra header in 'Key: Value' form (repeatable).
    #[arg(short = 'H', long = "header")]
    pub headers: Vec<String>,

    /// Live dashboard (requires the `tui` feature).
    #[arg(long)]
    pub tui: bool,

    /// Machine-readable report.
    #[arg(long)]
    pub json: bool,

    /// Disable ANSI color.
    #[arg(long)]
    pub no_color: bool,
}
