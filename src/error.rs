//! Typed domain errors for the core layer.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProbeError {
    /// Transport-level failure (DNS, connect, TLS, read) from `reqwest`.
    #[error("request error: {0}")]
    Http(#[from] reqwest::Error),

    /// Server returned a non-2xx status (not a context overflow).
    #[error("HTTP {status}: {body}")]
    Api { status: u16, body: String },

    /// Server refused the request because the conversation exceeded its context window.
    #[error("context overflow: {body}")]
    ContextOverflow { body: String },

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

/// Returns `true` when an HTTP error body looks like a context-window overflow.
///
/// vLLM / OpenAI-compatible servers return HTTP 400 with one of these phrases
/// when the accumulated prompt exceeds the model's maximum context length.
pub fn is_context_overflow(status: u16, body: &str) -> bool {
    if status != 400 && status != 413 {
        return false;
    }
    let lower = body.to_lowercase();
    [
        "maximum context length",
        "context_length_exceeded",
        "context window",
        "too many tokens",
        "exceeds the limit",
        "prompt is too long",
    ]
    .iter()
    .any(|pat| lower.contains(pat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_vllm_context_overflow() {
        let body = r#"{"object":"error","message":"This model's maximum context length is 8192 tokens. However, you requested 8500 tokens.","type":"invalid_request_error","code":"context_length_exceeded"}"#;
        assert!(is_context_overflow(400, body));
    }

    #[test]
    fn detects_openai_style() {
        assert!(is_context_overflow(400, "maximum context length is 4096"));
        assert!(is_context_overflow(413, "prompt is too long"));
    }

    #[test]
    fn ignores_other_400s() {
        assert!(!is_context_overflow(400, "invalid model name"));
        assert!(!is_context_overflow(429, "context_length_exceeded")); // wrong status
        assert!(!is_context_overflow(200, "context_length_exceeded")); // not an error
    }
}
