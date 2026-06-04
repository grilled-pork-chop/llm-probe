//! Validated, normalized run settings.
//!
//! `RunConfig` is the single source of truth the measurement core consumes; it
//! is built from `cli::Args` once, with the endpoint normalized (A.4) and all
//! values validated (A.3) *before* any request fires.

use crate::error::ProbeError;
use std::time::Duration;

/// Connect timeout is fixed (A.3); only the total timeout is user-tunable.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Everything the runner/client need, already validated and normalized.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Fully-resolved `…/v1/chat/completions` URL.
    pub endpoint: String,
    pub model: String,
    /// Number of requests; `0` means run indefinitely until interrupted.
    pub requests: usize,
    pub concurrency: usize,
    pub stream: bool,
    pub prompt: String,
    /// Multi-turn conversation messages in `(role, content)` order.
    /// Non-empty when `--message` flags were given; overrides `prompt`.
    pub messages: Vec<(String, String)>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub timeout: Duration,
    pub warmup: usize,
    pub api_key: Option<String>,
    /// Extra `K: V` headers, pre-split.
    pub headers: Vec<(String, String)>,
}

impl RunConfig {
    /// Validate raw settings (A.3) and normalize the endpoint (A.4).
    ///
    /// Returns `ProbeError::Config` with a clear message on the first problem.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        url: &str,
        model: String,
        requests: usize,
        concurrency: usize,
        stream: bool,
        prompt: String,
        max_tokens: u32,
        temperature: Option<f32>,
        timeout_secs: u64,
        warmup: usize,
        api_key: Option<String>,
        raw_headers: &[String],
        raw_messages: &[String],
    ) -> Result<Self, ProbeError> {
        // `requests == 0` is valid and means "run indefinitely".
        if concurrency < 1 {
            return Err(ProbeError::Config("concurrency must be >= 1".into()));
        }
        if timeout_secs == 0 {
            return Err(ProbeError::Config("timeout must be > 0".into()));
        }
        if max_tokens < 1 {
            return Err(ProbeError::Config("max-tokens must be >= 1".into()));
        }
        let endpoint = resolve_endpoint(url)?;
        let headers = parse_headers(raw_headers)?;
        let messages = parse_messages(raw_messages)?;
        Ok(Self {
            endpoint,
            model,
            requests,
            concurrency,
            stream,
            prompt,
            messages,
            max_tokens,
            temperature,
            timeout: Duration::from_secs(timeout_secs),
            warmup,
            api_key,
            headers,
        })
    }
}

/// Normalize a base or full endpoint into a chat-completions URL (A.4).
///
/// 1. trim a trailing `/`
/// 2. ends with `/chat/completions` → use as-is
/// 3. ends with `/v1` → append `/chat/completions`
/// 4. otherwise → append `/v1/chat/completions`
///
/// Must be an absolute `http`/`https` URL.
pub fn resolve_endpoint(input: &str) -> Result<String, ProbeError> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(ProbeError::Config("url must not be empty".into()));
    }
    let resolved = if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    };
    if !(resolved.starts_with("http://") || resolved.starts_with("https://")) {
        return Err(ProbeError::Config(format!(
            "url must be an absolute http(s) URL: {input}"
        )));
    }
    Ok(resolved)
}

/// Split `K: V` (or `K:V`) header strings; error on a missing colon.
fn parse_headers(raw: &[String]) -> Result<Vec<(String, String)>, ProbeError> {
    raw.iter()
        .map(|h| {
            let (k, v) = h.split_once(':').ok_or_else(|| {
                ProbeError::Config(format!("header must be in 'Key: Value' form: {h}"))
            })?;
            Ok((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Split `role: content` message strings; error on a missing colon.
fn parse_messages(raw: &[String]) -> Result<Vec<(String, String)>, ProbeError> {
    raw.iter()
        .map(|m| {
            let (role, content) = m.split_once(':').ok_or_else(|| {
                ProbeError::Config(format!("message must be in 'role: content' form: {m}"))
            })?;
            Ok((role.trim().to_string(), content.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_truth_table() {
        let cases = [
            (
                "http://localhost:8000",
                "http://localhost:8000/v1/chat/completions",
            ),
            (
                "http://localhost:8000/v1",
                "http://localhost:8000/v1/chat/completions",
            ),
            (
                "http://localhost:8000/v1/",
                "http://localhost:8000/v1/chat/completions",
            ),
            (
                "https://api.x.ai/v1/chat/completions",
                "https://api.x.ai/v1/chat/completions",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(resolve_endpoint(input).unwrap(), want, "input={input}");
        }
    }

    #[test]
    fn url_rejects_non_absolute() {
        assert!(resolve_endpoint("localhost:8000").is_err());
        assert!(resolve_endpoint("").is_err());
        assert!(resolve_endpoint("ftp://x/v1").is_err());
    }

    #[test]
    fn header_parsing() {
        let h = parse_headers(&["X-A: 1".into(), "X-B:2".into()]).unwrap();
        assert_eq!(
            h,
            vec![("X-A".into(), "1".into()), ("X-B".into(), "2".into())]
        );
        assert!(parse_headers(&["bad".into()]).is_err());
    }

    #[test]
    fn validation_rejects_bad_values() {
        let ok = RunConfig::build(
            "http://x",
            "m".into(),
            1,
            1,
            false,
            "p".into(),
            1,
            None,
            1,
            0,
            None,
            &[],
            &[],
        );
        assert!(ok.is_ok());
        // requests == 0 is valid and means "run indefinitely".
        let infinite = RunConfig::build(
            "http://x",
            "m".into(),
            0,
            1,
            false,
            "p".into(),
            1,
            None,
            1,
            0,
            None,
            &[],
            &[],
        );
        assert!(infinite.is_ok());
        let bad_concurrency = RunConfig::build(
            "http://x",
            "m".into(),
            1,
            0,
            false,
            "p".into(),
            1,
            None,
            1,
            0,
            None,
            &[],
            &[],
        );
        assert!(bad_concurrency.is_err());
        let bad_timeout = RunConfig::build(
            "http://x",
            "m".into(),
            1,
            1,
            false,
            "p".into(),
            1,
            None,
            0,
            0,
            None,
            &[],
            &[],
        );
        assert!(bad_timeout.is_err());
    }
}
