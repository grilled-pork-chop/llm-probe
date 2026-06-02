//! Typed domain errors for the core layer.
//!
//! Per §11: per-request failures are *recorded into* a `RequestOutcome`, never
//! propagated to abort the batch. `ProbeError` is the shape captured there and
//! also covers setup-time failures (URL parsing, config).

use thiserror::Error;

/// Errors produced by the measurement core.
#[derive(Debug, Error)]
pub enum ProbeError {
    /// Transport-level failure (DNS, connect, TLS, read) from `reqwest`.
    #[error("request error: {0}")]
    Http(#[from] reqwest::Error),

    /// Server returned a non-2xx status. `body` is truncated for display.
    #[error("HTTP {status}: {body}")]
    Api { status: u16, body: String },

    /// Response body failed to deserialize.
    #[error("decode error: {0}")]
    Decode(#[from] serde_json::Error),

    /// Streaming-specific failure (malformed SSE line, broken stream).
    #[error("stream error: {0}")]
    Stream(String),

    /// Per-request deadline exceeded.
    #[error("request timed out")]
    Timeout,

    /// Invalid configuration or un-parseable URL, surfaced before any request.
    #[error("{0}")]
    Config(String),
}
