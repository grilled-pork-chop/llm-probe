//! Validated, normalized run settings.

use crate::error::ProbeError;
use std::time::Duration;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Everything the runner/client need, already validated and normalized.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Fully-resolved `.../v1/chat/completions` URL.
    pub endpoint: String,
    pub model: String,
    /// Total conversations to run across all slots; `0` = run forever.
    pub conversations: usize,
    /// Number of concurrent conversation slots.
    pub concurrency: usize,
    pub stream: bool,
    /// Seed user message override. When `prompt_is_default` is true the pool
    /// seed is used instead and this field is ignored.
    pub prompt: String,
    /// True when --prompt was not explicitly set — the pool provides the seed.
    pub prompt_is_default: bool,
    /// Per-conversation turn cap; `0` = unlimited.
    pub max_turns_per_conv: usize,
    /// Initial conversation seed as `(role, content)` pairs.
    /// Non-empty when `--message` flags were given; overrides `prompt`.
    pub messages: Vec<(String, String)>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub timeout: Duration,
    pub api_key: Option<String>,
    pub headers: Vec<(String, String)>,
}

impl RunConfig {
    /// Resolved initial conversation seed: the explicit `--message` pairs when
    /// given, otherwise the single `--prompt` user turn. Single source of truth
    /// for both the runner's growth loop and the replay/TUI request view.
    pub fn seed_messages(&self) -> Vec<(String, String)> {
        if self.messages.is_empty() {
            vec![("user".into(), self.prompt.clone())]
        } else {
            self.messages.clone()
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build(
        url: &str,
        model: String,
        conversations: usize,
        concurrency: usize,
        stream: bool,
        prompt: String,
        prompt_is_default: bool,
        max_turns_per_conv: usize,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
        timeout_secs: u64,
        api_key: Option<String>,
        raw_headers: &[String],
        raw_messages: &[String],
    ) -> Result<Self, ProbeError> {
        if concurrency < 1 {
            return Err(ProbeError::Config("concurrency must be >= 1".into()));
        }
        if timeout_secs == 0 {
            return Err(ProbeError::Config("timeout must be > 0".into()));
        }
        if matches!(max_tokens, Some(0)) {
            return Err(ProbeError::Config("max-tokens must be >= 1 when set".into()));
        }
        let endpoint = resolve_endpoint(url)?;
        let headers = parse_kv("header", ':', raw_headers)?;
        let messages = parse_kv("message", ':', raw_messages)?;
        Ok(Self {
            endpoint,
            model,
            conversations,
            concurrency,
            stream,
            prompt,
            prompt_is_default,
            max_turns_per_conv,
            messages,
            max_tokens,
            temperature,
            timeout: Duration::from_secs(timeout_secs),
            api_key,
            headers,
        })
    }
}

/// Normalize a base or full endpoint into a chat-completions URL.
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

/// Split `key: value` strings on the first occurrence of `sep`.
fn parse_kv(kind: &str, sep: char, raw: &[String]) -> Result<Vec<(String, String)>, ProbeError> {
    raw.iter()
        .map(|s| {
            let (k, v) = s.split_once(sep).ok_or_else(|| {
                ProbeError::Config(format!(
                    "{kind} must be in 'key{sep} value' form: {s}"
                ))
            })?;
            Ok((k.trim().to_string(), v.trim().to_string()))
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
    fn kv_parsing() {
        let h = parse_kv("header", ':', &["X-A: 1".into(), "X-B:2".into()]).unwrap();
        assert_eq!(
            h,
            vec![("X-A".into(), "1".into()), ("X-B".into(), "2".into())]
        );
        assert!(parse_kv("header", ':', &["bad".into()]).is_err());
    }

    #[test]
    fn build_accepts_valid_config() {
        let cfg = RunConfig::build(
            "http://x", "m".into(), 0, 1, false,
            "p".into(), true, 0, None, None, 1, None, &[], &[],
        );
        assert!(cfg.is_ok());
    }

    #[test]
    fn build_rejects_bad_values() {
        assert!(RunConfig::build(
            "http://x", "m".into(), 0, 0, false,
            "p".into(), true, 0, None, None, 1, None, &[], &[],
        ).is_err()); // concurrency=0

        assert!(RunConfig::build(
            "http://x", "m".into(), 0, 1, false,
            "p".into(), true, 0, None, None, 0, None, &[], &[],
        ).is_err()); // timeout=0
    }
}
