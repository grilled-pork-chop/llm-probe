//! Build and send a single conversation turn, capturing the result into a
//! `TurnOutcome`. Errors are recorded, never propagated: one bad turn must not
//! abort the whole conversation.

use crate::config::{CONNECT_TIMEOUT, RunConfig};
use crate::error::{ProbeError, is_context_overflow};
use crate::metrics::{ErrorKind, TurnOutcome};
use crate::runner::RunEvent;
use crate::types::{ChatChunk, ChatRequest, ChatResponse, Message, StreamOptions, Usage};
use futures_util::StreamExt;
use reqwest::Client;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

const ERR_BODY_LIMIT: usize = 512;
const BODY_LIMIT: usize = 8192;

fn cap_body(s: String) -> String {
    if s.chars().count() <= BODY_LIMIT {
        return s;
    }
    let mut out: String = s.chars().take(BODY_LIMIT).collect();
    out.push_str("\n… (truncated)");
    out
}

pub fn build_client(cfg: &RunConfig) -> Result<Client, ProbeError> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(cfg.timeout)
        .build()
        .map_err(ProbeError::from)
}

/// Send one conversation turn and return its outcome. Never returns `Err`.
///
/// `messages` is the full conversation history to send (already includes the
/// new user turn appended by the caller).
pub async fn send_turn(
    client: &Client,
    cfg: &RunConfig,
    conv_id: usize,
    turn_idx: usize,
    messages: &[(String, String)],
    tx: Option<&UnboundedSender<RunEvent>>,
) -> TurnOutcome {
    if let Some(tx) = tx {
        let _ = tx.send(RunEvent::TurnStarted { conv_id, turn_idx });
    }
    let t0 = Instant::now();
    match send(client, cfg, conv_id, turn_idx, messages, tx, t0).await {
        Ok(outcome) => outcome,
        Err(err) => {
            let kind = ErrorKind::from_probe(&err);
            TurnOutcome::failed(conv_id, turn_idx, t0.elapsed(), kind)
        }
    }
}

async fn send(
    client: &Client,
    cfg: &RunConfig,
    conv_id: usize,
    turn_idx: usize,
    messages: &[(String, String)],
    tx: Option<&UnboundedSender<RunEvent>>,
    t0: Instant,
) -> Result<TurnOutcome, ProbeError> {
    let msg_refs: Vec<Message> = messages
        .iter()
        .map(|(role, content)| Message { role, content })
        .collect();

    let body = ChatRequest {
        model: &cfg.model,
        messages: msg_refs,
        stream: cfg.stream,
        max_tokens: Some(cfg.max_tokens),
        temperature: cfg.temperature,
        stream_options: cfg.stream.then_some(StreamOptions { include_usage: true }),
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
        let raw = resp.text().await.unwrap_or_default();
        let truncated: String = raw.chars().take(ERR_BODY_LIMIT).collect();
        if is_context_overflow(status.as_u16(), &truncated) {
            return Err(ProbeError::ContextOverflow { body: truncated });
        }
        return Err(ProbeError::Api {
            status: status.as_u16(),
            body: truncated,
        });
    }

    if cfg.stream {
        stream_response(resp, conv_id, turn_idx, tx, t0).await
    } else {
        non_streaming(resp, conv_id, turn_idx, t0).await
    }
}

async fn non_streaming(
    resp: reqwest::Response,
    conv_id: usize,
    turn_idx: usize,
    t0: Instant,
) -> Result<TurnOutcome, ProbeError> {
    let parsed: ChatResponse = resp.json().await?;
    let e2e = t0.elapsed();
    let (completion_tokens, prompt_tokens) = match parsed.usage {
        Some(u) => (Some(u.completion_tokens), Some(u.prompt_tokens)),
        None => (None, None),
    };
    let tps = match completion_tokens {
        Some(c) if c > 0 => positive_ratio(f64::from(c), e2e),
        _ => None,
    };
    let reply = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .filter(|s| !s.is_empty())
        .map(cap_body);
    Ok(TurnOutcome {
        conv_id,
        turn_idx,
        prompt_tokens,
        completion_tokens,
        e2e,
        ttft: None,
        tpot_ms: None,
        itl_ms: None,
        tps,
        success: true,
        error: None,
        reply,
    })
}

#[derive(Debug, PartialEq)]
enum SseEvent {
    Content(String),
    Usage(Usage),
    Done,
}

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
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
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

async fn stream_response(
    resp: reqwest::Response,
    conv_id: usize,
    turn_idx: usize,
    tx: Option<&UnboundedSender<RunEvent>>,
    t0: Instant,
) -> Result<TurnOutcome, ProbeError> {
    let mut byte_stream = resp.bytes_stream();
    let mut decoder = SseDecoder::default();
    let mut t_first: Option<Instant> = None;
    let mut t_last: Option<Instant> = None;
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
                    if t_first.is_none() {
                        t_first = Some(now);
                        if let Some(tx) = tx {
                            let _ = tx.send(RunEvent::TurnFirstToken {
                                conv_id,
                                turn_idx,
                                ttft: now.saturating_duration_since(t0),
                            });
                        }
                    }
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
        // Single-chunk: fall back to e2e − TTFT as the decode window.
        (Some(_), Some(_)) => ttft
            .map(|t| e2e.saturating_sub(t))
            .filter(|d| !d.is_zero()),
        _ => None,
    };
    let gen_secs = gen_time.map(|d| d.as_secs_f64()).filter(|s| *s > 0.0);
    let completion_tokens = usage.map(|u| u.completion_tokens);
    let prompt_tokens = usage.map(|u| u.prompt_tokens);

    let tps = match (completion_tokens, gen_secs) {
        (Some(c), Some(s)) if c > 0 => Some(f64::from(c) / s),
        _ => None,
    };
    // TPOT: (e2e − TTFT) / (completion_tokens − 1)
    let tpot_ms = match (completion_tokens, ttft) {
        (Some(c), Some(tf)) if c > 1 => {
            let decode = e2e.saturating_sub(tf).as_secs_f64() * 1000.0;
            let steps = f64::from(c - 1);
            (steps > 0.0).then(|| decode / steps)
        }
        _ => None,
    };
    let itl_ms = match (completion_tokens, gen_time) {
        (Some(c), Some(g)) if c > 1 => Some(g.as_secs_f64() * 1000.0 / f64::from(c - 1)),
        _ => None,
    };
    let reply = (!reply.is_empty()).then(|| cap_body(reply));

    Ok(TurnOutcome {
        conv_id,
        turn_idx,
        prompt_tokens,
        completion_tokens,
        e2e,
        ttft,
        tpot_ms,
        itl_ms,
        tps,
        success: true,
        error: None,
        reply,
    })
}

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
        assert!(d.feed(b"data: {\"choices\":[{\"delta\":{\"con").unwrap().is_empty());
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
