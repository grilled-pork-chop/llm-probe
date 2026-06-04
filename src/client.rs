//! Build and send a single request, capturing the result into a
//! `RequestOutcome`. Errors are recorded, never propagated (§6, §11): one bad
//! request must not abort the batch.

use crate::config::{CONNECT_TIMEOUT, RunConfig};
use crate::error::ProbeError;
use crate::metrics::{ErrorKind, RequestOutcome};
use crate::runner::RunEvent;
use crate::types::{ChatChunk, ChatRequest, ChatResponse, Message, StreamOptions, Usage};
use futures_util::StreamExt;
use reqwest::Client;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

/// Max chars of an error body retained for display.
const ERR_BODY_LIMIT: usize = 512;

/// Max chars of a captured response body kept for the inspector.
const BODY_LIMIT: usize = 8192;

/// Truncate `s` to at most `BODY_LIMIT` chars, appending an ellipsis note.
fn cap_body(s: String) -> String {
    if s.chars().count() <= BODY_LIMIT {
        return s;
    }
    let mut out: String = s.chars().take(BODY_LIMIT).collect();
    out.push_str("\n… (truncated)");
    out
}

/// Build the one shared, connection-pooled client (cloned per request).
pub fn build_client(cfg: &RunConfig) -> Result<Client, ProbeError> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(cfg.timeout)
        .build()
        .map_err(ProbeError::from)
}

/// Send one request and return its outcome. Never returns `Err`.
pub async fn run_one(
    client: &Client,
    cfg: &RunConfig,
    id: usize,
    tx: Option<&UnboundedSender<RunEvent>>,
) -> RequestOutcome {
    if let Some(tx) = tx {
        let _ = tx.send(RunEvent::Started { id });
    }
    let t0 = Instant::now();
    match send(client, cfg, id, tx, t0).await {
        Ok(outcome) => outcome,
        Err(err) => {
            // Surface the server's error body in the inspector when we have one.
            let body = match &err {
                ProbeError::Api { body, .. } if !body.is_empty() => Some(body.clone()),
                _ => None,
            };
            RequestOutcome::failed(id, t0.elapsed(), ErrorKind::from_probe(&err), body)
        }
    }
}

/// The fallible body of a request; the caller wraps any `Err` into a failed
/// outcome so the batch keeps running.
async fn send(
    client: &Client,
    cfg: &RunConfig,
    id: usize,
    tx: Option<&UnboundedSender<RunEvent>>,
    t0: Instant,
) -> Result<RequestOutcome, ProbeError> {
    let messages: Vec<Message> = if cfg.messages.is_empty() {
        vec![Message {
            role: "user",
            content: &cfg.prompt,
        }]
    } else {
        cfg.messages
            .iter()
            .map(|(role, content)| Message { role, content })
            .collect()
    };
    let body = ChatRequest {
        model: &cfg.model,
        messages,
        stream: cfg.stream,
        max_tokens: Some(cfg.max_tokens),
        temperature: cfg.temperature,
        stream_options: cfg.stream.then_some(StreamOptions {
            include_usage: true,
        }),
    };

    let mut req = client.post(&cfg.endpoint).json(&body);
    if let Some(key) = &cfg.api_key {
        req = req.bearer_auth(key);
    }
    for (k, v) in &cfg.headers {
        req = req.header(k, v);
    }

    let resp = req.send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let truncated: String = body.chars().take(ERR_BODY_LIMIT).collect();
        return Err(ProbeError::Api {
            status: status.as_u16(),
            body: truncated,
        });
    }

    if cfg.stream {
        stream_response(resp, id, tx, t0).await
    } else {
        non_streaming(resp, id, t0).await
    }
}

/// Await the full body, parse it, and assemble the outcome (§6 step 4).
async fn non_streaming(
    resp: reqwest::Response,
    id: usize,
    t0: Instant,
) -> Result<RequestOutcome, ProbeError> {
    let parsed: ChatResponse = resp.json().await?;
    let e2e = t0.elapsed();
    let (completion_tokens, prompt_tokens) = match parsed.usage {
        Some(u) => (Some(u.completion_tokens), Some(u.prompt_tokens)),
        None => (None, None),
    };
    // Non-streaming TPS uses e2e (no per-token timing); reports label it as such.
    let tps = match completion_tokens {
        Some(c) if c > 0 => positive_ratio(f64::from(c), e2e),
        _ => None,
    };
    let body = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .filter(|s| !s.is_empty())
        .map(cap_body);
    Ok(RequestOutcome {
        id,
        success: true,
        e2e,
        ttft: None,
        gen_time: None,
        completion_tokens,
        prompt_tokens,
        tps,
        itl_ms: None,
        max_gap_ms: None,
        error: None,
        body,
    })
}

/// A decoded SSE event from the chat-completions stream.
#[derive(Debug, PartialEq)]
enum SseEvent {
    /// A non-empty content delta arrived (the token text, for assertions).
    Content(String),
    /// The final usage-bearing chunk.
    Usage(Usage),
    /// `[DONE]` sentinel; the caller stops consuming.
    Done,
}

/// Incremental SSE line framer for chat-completion streams (A.6).
///
/// Pure and synchronous so framing is unit-testable in isolation from async I/O
/// and timing. `feed` accepts arbitrary byte chunks (payloads split across reads
/// are buffered) and returns the events found, in order, stopping at `[DONE]`.
#[derive(Default)]
struct SseDecoder {
    buf: Vec<u8>,
}

impl SseDecoder {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, ProbeError> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = std::str::from_utf8(&line_bytes[..line_bytes.len() - 1])
                .map_err(|_| ProbeError::Stream("invalid utf-8 in stream".into()))?;
            let line = line.strip_suffix('\r').unwrap_or(line);

            // Blank lines and `:` comments carry no payload.
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            // Only `data:` lines carry payload; ignore other SSE fields.
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.strip_prefix(' ').unwrap_or(payload);
            if payload == "[DONE]" {
                events.push(SseEvent::Done);
                break;
            }

            let parsed: ChatChunk = serde_json::from_str(payload)
                .map_err(|e| ProbeError::Stream(format!("bad SSE chunk: {e}")))?;
            // `choices` may be empty on the final usage-bearing chunk (A.6).
            if let Some(choice) = parsed.choices.first()
                && let Some(content) = &choice.delta.content
                && !content.is_empty()
            {
                events.push(SseEvent::Content(content.clone()));
            }
            if let Some(u) = parsed.usage {
                events.push(SseEvent::Usage(u));
            }
        }
        Ok(events)
    }
}

/// Consume an SSE stream of chat-completion chunks (A.6).
///
/// Tracks first/last content-token timestamps for TTFT, decode window, ITL, and
/// max inter-token gap; captures `usage` from the final chunk.
async fn stream_response(
    resp: reqwest::Response,
    id: usize,
    tx: Option<&UnboundedSender<RunEvent>>,
    t0: Instant,
) -> Result<RequestOutcome, ProbeError> {
    let mut byte_stream = resp.bytes_stream();
    let mut decoder = SseDecoder::default();
    let mut t_first: Option<Instant> = None;
    let mut t_last: Option<Instant> = None;
    let mut last_content: Option<Instant> = None;
    let mut max_gap_ms = 0.0_f64;
    let mut usage: Option<Usage> = None;
    let mut reply = String::new();

    'outer: while let Some(chunk) = byte_stream.next().await {
        for event in decoder.feed(&chunk?)? {
            match event {
                SseEvent::Content(text) => {
                    if reply.len() < BODY_LIMIT {
                        reply.push_str(&text);
                    }
                    let now = Instant::now();
                    match (t_first, last_content) {
                        (None, _) => {
                            t_first = Some(now);
                            if let Some(tx) = tx {
                                let _ = tx.send(RunEvent::FirstToken {
                                    id,
                                    ttft: now.saturating_duration_since(t0),
                                });
                            }
                        }
                        (Some(_), Some(prev)) => {
                            let gap = now.saturating_duration_since(prev).as_secs_f64() * 1000.0;
                            max_gap_ms = max_gap_ms.max(gap);
                        }
                        _ => {}
                    }
                    last_content = Some(now);
                    t_last = Some(now);
                }
                SseEvent::Usage(u) => usage = Some(u),
                SseEvent::Done => break 'outer,
            }
        }
    }

    let e2e = t0.elapsed();
    let ttft = t_first.map(|t| t.saturating_duration_since(t0));
    let gen_time = match (t_first, t_last) {
        (Some(f), Some(l)) if l > f => Some(l.saturating_duration_since(f)),
        // Single-chunk response: all content arrived in one SSE event so
        // t_first == t_last. Fall back to e2e minus TTFT as the decode window
        // so per-request TPS is still defined (common with GLM via vLLM).
        (Some(_), Some(_)) => ttft
            .map(|t| e2e.saturating_sub(t))
            .filter(|d| !d.is_zero()),
        _ => None,
    };
    let gen_secs = gen_time.map(|d| d.as_secs_f64()).filter(|s| *s > 0.0);
    let completion_tokens = usage.map(|u| u.completion_tokens);
    let prompt_tokens = usage.map(|u| u.prompt_tokens);

    // Decode throughput excludes TTFT/queueing (§2). None when undefined (A.9).
    let tps = match (completion_tokens, gen_secs) {
        (Some(c), Some(s)) if c > 0 => Some(f64::from(c) / s),
        _ => None,
    };
    let itl_ms = match (completion_tokens, gen_time) {
        (Some(c), Some(g)) if c > 1 => Some(g.as_secs_f64() * 1000.0 / f64::from(c - 1)),
        _ => None,
    };
    let max_gap_ms = (max_gap_ms > 0.0).then_some(max_gap_ms);
    let body = (!reply.is_empty()).then(|| cap_body(reply));

    Ok(RequestOutcome {
        id,
        success: true,
        e2e,
        ttft,
        gen_time,
        completion_tokens,
        prompt_tokens,
        tps,
        itl_ms,
        max_gap_ms,
        error: None,
        body,
    })
}

/// `numer / dur_secs`, or `None` when the denominator is non-positive (A.9).
fn positive_ratio(numer: f64, dur: Duration) -> Option<f64> {
    let secs = dur.as_secs_f64();
    if secs > 0.0 { Some(numer / secs) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contents(events: &[SseEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                SseEvent::Content(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn parses_data_lines_and_done() {
        let mut d = SseDecoder::default();
        let input = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n",
            "data: [DONE]\n",
        );
        let events = d.feed(input.as_bytes()).unwrap();
        assert_eq!(contents(&events), vec!["Hello", " world"]);
        assert_eq!(events.last(), Some(&SseEvent::Done));
    }

    #[test]
    fn skips_role_only_priming_chunk() {
        // First chunk has a role but no content; it must not count as a token.
        let mut d = SseDecoder::default();
        let input = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n",
        );
        let events = d.feed(input.as_bytes()).unwrap();
        assert_eq!(contents(&events), vec!["hi"]);
    }

    #[test]
    fn handles_empty_choices_usage_chunk() {
        // The final usage chunk carries an empty `choices` array (A.6).
        let mut d = SseDecoder::default();
        let input =
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":7}}\n";
        let events = d.feed(input.as_bytes()).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            SseEvent::Usage(u) => {
                assert_eq!(u.completion_tokens, 7);
                assert_eq!(u.prompt_tokens, 3);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn reassembles_payload_split_across_reads() {
        let mut d = SseDecoder::default();
        // A single data line delivered in three byte slices, mid-token and mid-newline.
        assert!(
            d.feed(b"data: {\"choices\":[{\"delta\":{\"con")
                .unwrap()
                .is_empty()
        );
        assert!(d.feed(b"tent\":\"split\"}}]}").unwrap().is_empty());
        let events = d.feed(b"\n").unwrap();
        assert_eq!(contents(&events), vec!["split"]);
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let mut d = SseDecoder::default();
        let input = concat!(
            ": keep-alive comment\n",
            "\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n",
        );
        let events = d.feed(input.as_bytes()).unwrap();
        assert_eq!(contents(&events), vec!["x"]);
    }

    #[test]
    fn malformed_json_is_stream_error() {
        let mut d = SseDecoder::default();
        let err = d.feed(b"data: {not json}\n").unwrap_err();
        assert!(matches!(err, ProbeError::Stream(_)));
    }

    #[test]
    fn strips_crlf_line_endings() {
        let mut d = SseDecoder::default();
        let input = "data: {\"choices\":[{\"delta\":{\"content\":\"crlf\"}}]}\r\ndata: [DONE]\r\n";
        let events = d.feed(input.as_bytes()).unwrap();
        assert_eq!(contents(&events), vec!["crlf"]);
        assert_eq!(events.last(), Some(&SseEvent::Done));
    }
}
