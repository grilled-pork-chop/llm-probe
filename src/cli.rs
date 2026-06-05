//! Command-line surface.

use clap::Parser;

/// Default first user message when no -M flags are given.
pub const DEFAULT_PROMPT: &str = "Write three sentences about the history of computing.";

/// Default follow-up message appended every turn to grow the conversation.
pub const DEFAULT_TURN: &str = "Please continue.";

#[derive(Debug, Parser)]
#[command(
    name = "llmprobe",
    about = "Grow conversations to the context limit on an OpenAI-compatible chat endpoint",
    long_about = "Runs C concurrent conversation slots (-c), each growing a conversation \
turn-by-turn until the server refuses with a context-length error. \
Collect TTFT, TPOT, TPS, and context-window stats per turn, per conversation, \
and in aggregate. Save runs with --output and replay them interactively with --replay.",
    version
)]
pub struct Args {
    /// Base or full endpoint. Appends /v1/chat/completions if absent.
    #[arg(short, long, required_unless_present = "replay")]
    pub url: Option<String>,

    /// Model name.
    #[arg(short, long, required_unless_present = "replay")]
    pub model: Option<String>,

    /// Total conversations to complete across all slots; 0 = run forever.
    #[arg(short = 'n', long = "conversations", default_value_t = 0)]
    pub conversations: usize,

    /// Concurrent conversation slots (virtual users).
    #[arg(short, long, default_value_t = 1)]
    pub concurrency: usize,

    /// Enable streaming (measures TTFT and TPOT; strongly recommended).
    #[arg(long)]
    pub stream: bool,

    /// Initial user message (seed for every conversation).
    #[arg(short, long, default_value = DEFAULT_PROMPT)]
    pub prompt: String,

    /// User message appended every grow step. Defaults to "Please continue."
    #[arg(long, default_value = DEFAULT_TURN)]
    pub turn: String,

    /// Stop a conversation after this many turns regardless of context limit (0 = unlimited).
    #[arg(long = "max-turns", default_value_t = 0)]
    pub max_turns: usize,

    /// Cap output tokens per turn (omitted when unset — server decides).
    #[arg(long)]
    pub max_tokens: Option<u32>,

    /// Sampling temperature (omitted from the request when unset).
    #[arg(long)]
    pub temperature: Option<f32>,

    /// Per-turn request timeout in seconds.
    #[arg(long, default_value_t = 60)]
    pub timeout: u64,

    /// Bearer token (falls back to $OPENAI_API_KEY).
    #[arg(long, env = "OPENAI_API_KEY")]
    pub api_key: Option<String>,

    /// Extra header in 'Key: Value' form (repeatable).
    #[arg(short = 'H', long = "header")]
    pub headers: Vec<String>,

    /// Seed conversation turn in 'role: content' form (repeatable, in order).
    /// When given, overrides --prompt.
    #[arg(short = 'M', long = "message")]
    pub messages: Vec<String>,

    /// Live TUI dashboard (requires the `tui` feature).
    #[arg(long)]
    pub tui: bool,

    /// Machine-readable JSON report.
    #[arg(long)]
    pub json: bool,

    /// Disable ANSI colour.
    #[arg(long)]
    pub no_color: bool,

    /// Write the completed run to FILE as JSON (can be reopened with --replay).
    #[arg(long)]
    pub output: Option<String>,

    /// Load a saved run from FILE and open the interactive view (no HTTP requests made).
    #[arg(long, conflicts_with_all = ["url", "model"])]
    pub replay: Option<String>,
}
