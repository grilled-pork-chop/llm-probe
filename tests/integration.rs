//! End-to-end integration tests for the grow-mode runner.
//!
//! Tests drive the real client/runner stack against local mock servers:
//! - wiremock for simple request/response sequencing
//! - axum for handlers that need to inspect or count request bodies

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use futures_util::StreamExt;
use futures_util::stream;

use llmprobe::config::RunConfig;
use llmprobe::metrics::{ErrorKind, TerminalReason, aggregate};
use llmprobe::prompts::PromptSampler;
use llmprobe::runner::run_grow;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Shared helpers ────────────────────────────────────────────────────────────

fn make_cfg(
    base_url: &str,
    conversations: usize,
    concurrency: usize,
    stream: bool,
    max_turns: usize,
) -> RunConfig {
    RunConfig::build(
        base_url,
        "test-model".into(),
        conversations,
        concurrency,
        stream,
        None,
        max_turns,
        Some(64),
        None,
        10,
        None,
        &[],
    )
    .expect("valid test config")
}

async fn spawn_app(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    format!("http://{addr}")
}

fn success_body() -> serde_json::Value {
    serde_json::json!({
        "choices": [{ "message": { "content": "assistant reply" } }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
    })
}

fn overflow_body() -> serde_json::Value {
    serde_json::json!({
        "error": { "message": "maximum context length exceeded" }
    })
}

// ── Test 1: context-limit terminates conversation ─────────────────────────────

#[tokio::test]
async fn context_limit_terminates_conversation() {
    let server = MockServer::start().await;

    // First 3 requests succeed
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
        .up_to_n_times(3)
        .mount(&server)
        .await;

    // 4th request onwards triggers context overflow
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(overflow_body()))
        .mount(&server)
        .await;

    let cfg = make_cfg(&server.uri(), 1, 1, false, 0);
    let result = run_grow(&cfg, None, None).await.expect("run completes");

    assert_eq!(result.conversations.len(), 1, "exactly one conversation");
    let conv = &result.conversations[0];
    assert_eq!(
        conv.terminal,
        TerminalReason::ContextLimit,
        "should terminate with ContextLimit"
    );
    let ok_count = conv.turns.iter().filter(|t| t.success).count();
    assert_eq!(ok_count, 3, "3 successful turns before overflow");
    // The 4th turn (the overflow attempt) is recorded as failed
    assert_eq!(conv.turns.len(), 4, "3 ok + 1 failed overflow turn");

    // The expected context-overflow turn must not count against the success
    // rate: it is excluded from the denominator, so a clean grow run is 100%.
    let report = aggregate(&result);
    assert_eq!(report.turns.ok_turns, 3);
    assert_eq!(
        report.turns.total_turns, 3,
        "overflow turn excluded from denominator"
    );
    assert!(
        (report.turns.success_rate - 1.0).abs() < 0.001,
        "100% success rate on a clean grow run"
    );
}

// ── Test 2: conversation history grows each turn ──────────────────────────────

type MessageLog = Arc<tokio::sync::Mutex<Vec<usize>>>;

async fn recording_handler(
    State(log): State<MessageLog>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Response {
    let msg_count = body["messages"].as_array().map(|a| a.len()).unwrap_or(0);

    let mut lock = log.lock().await;
    lock.push(msg_count);
    let call_n = lock.len();
    drop(lock);

    // Overflow on the 4th call to keep the test finite
    if call_n > 3 {
        return (
            StatusCode::BAD_REQUEST,
            axum::extract::Json(overflow_body()),
        )
            .into_response();
    }
    axum::extract::Json(serde_json::json!({
        "choices": [{ "message": { "content": "assistant reply" } }],
        "usage": { "prompt_tokens": msg_count, "completion_tokens": 5 }
    }))
    .into_response()
}

#[tokio::test]
async fn conversation_history_grows_each_turn() {
    let log: MessageLog = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/v1/chat/completions", post(recording_handler))
        .with_state(log.clone());
    let url = spawn_app(app).await;

    let cfg = make_cfg(&url, 1, 1, false, 0);
    run_grow(&cfg, None, None).await.expect("run completes");

    let counts = log.lock().await;
    // Turn 0: [system, user(seed)]                                 → 2 messages
    // Turn 1: [system, user, assistant, user(pool)]                → 4 messages
    // Turn 2: [system, user, asst, user, asst, user(pool)]        → 6 messages
    // Turn 3 (overflow): grows by 2 again                          → 8 messages
    assert_eq!(
        *counts,
        vec![2, 4, 6, 8],
        "messages array must grow by 2 each turn (one assistant + one pool user turn)"
    );
}

// ── Test 3: concurrent slots produce independent conversations ────────────────

#[tokio::test]
async fn concurrent_slots_complete_independently() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
        .mount(&server)
        .await;

    // c=2 slots, n=4 conversations, max_turns=1 so each conv is just 1 turn
    let cfg = RunConfig::build(
        &server.uri(),
        "test-model".into(),
        4,
        2,
        false,
        None,
        1,
        Some(64),
        None,
        10,
        None,
        &[],
    )
    .expect("cfg");

    let result = run_grow(&cfg, None, None).await.expect("run completes");

    assert_eq!(result.conversations.len(), 4, "4 conversations total");

    // All conversation ids should be unique
    let mut ids: Vec<usize> = result.conversations.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![0, 1, 2, 3], "conversation ids 0–3 present");

    // Each conversation has exactly 1 turn (max-turns=1) and ends with MaxTurns
    for conv in &result.conversations {
        assert_eq!(conv.turns.len(), 1, "conv {} has 1 turn", conv.id);
        assert_eq!(
            conv.terminal,
            TerminalReason::MaxTurns,
            "conv {} should end at max-turns",
            conv.id
        );
    }

    // Slots stay within bounds
    let slot_ids: Vec<usize> = result.conversations.iter().map(|c| c.slot).collect();
    assert!(
        slot_ids.iter().all(|&s| s < 2),
        "all slot ids must be < c=2"
    );
}

// ── Test 4: max-turns cap stops conversation ──────────────────────────────────

#[tokio::test]
async fn max_turns_cap_stops_conversation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(success_body()))
        .mount(&server)
        .await;

    // max_turns=3: each conversation should run exactly 3 turns then stop
    let cfg = make_cfg(&server.uri(), 2, 1, false, 3);
    let result = run_grow(&cfg, None, None).await.expect("run completes");

    assert_eq!(result.conversations.len(), 2);
    for conv in &result.conversations {
        assert_eq!(
            conv.terminal,
            TerminalReason::MaxTurns,
            "conv {} should end at max-turns cap",
            conv.id
        );
        assert_eq!(
            conv.turns.len(),
            3,
            "conv {} should have exactly 3 turns",
            conv.id
        );
        assert!(
            conv.turns.iter().all(|t| t.success),
            "all turns in conv {} should succeed",
            conv.id
        );
    }

    // Aggregate stats should reflect 6 successful turns total
    let report = aggregate(&result);
    assert_eq!(
        report.turns.ok_turns, 6,
        "6 ok turns total (2 convs × 3 turns)"
    );
    assert_eq!(report.turns.total_turns, 6);
    assert!(
        (report.turns.success_rate - 1.0).abs() < 0.001,
        "100% success rate"
    );
}

// ── SSE streaming test ────────────────────────────────────────────────────────

async fn sse_handler() -> impl IntoResponse {
    let mut frames: Vec<String> = Vec::new();
    frames.push("data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n".to_string());
    for _ in 0..8 {
        frames.push("data: {\"choices\":[{\"delta\":{\"content\":\"tok \"}}]}\n\n".to_string());
    }
    frames.push(
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":8}}\n\n"
            .to_string(),
    );
    frames.push("data: [DONE]\n\n".to_string());

    let body = stream::iter(frames).then(|f| async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok::<Bytes, Infallible>(Bytes::from(f))
    });
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(body),
    )
}

#[tokio::test]
async fn streaming_grow_populates_ttft_and_tps() {
    // Use max_turns=2 so the test terminates quickly
    let app = Router::new().route("/v1/chat/completions", post(sse_handler));
    let url = spawn_app(app).await;

    let cfg = make_cfg(&url, 1, 1, true, 2);
    let result = run_grow(&cfg, None, None).await.expect("run completes");

    assert_eq!(result.conversations.len(), 1);
    let conv = &result.conversations[0];
    assert_eq!(conv.terminal, TerminalReason::MaxTurns);

    for turn in conv.turns.iter().filter(|t| t.success) {
        assert!(
            turn.ttft.is_some(),
            "TTFT must be present for streaming turns"
        );
        assert!(
            turn.tps.is_some(),
            "TPS must be computed for streaming turns"
        );
        assert_eq!(turn.completion_tokens, Some(8));
    }

    let report = aggregate(&result);
    assert!(report.turns.ttft.is_some(), "aggregate TTFT present");
    assert!(report.turns.tps.is_some(), "aggregate TPS present");
}

// ── Test 6: seed reproducibility ─────────────────────────────────────────────

#[test]
fn seed_reproducibility() {
    let mut a = PromptSampler::new(Some(42));
    let mut b = PromptSampler::new(Some(42));

    assert_eq!(
        a.system(),
        b.system(),
        "system prompts must match for same seed"
    );
    assert_eq!(a.seed(), b.seed(), "seed turns must match for same seed");

    for i in 0..10 {
        assert_eq!(
            a.next_followup(),
            b.next_followup(),
            "followup {i} must match for same seed"
        );
    }
}

// ── Test 7: timeout error classification ─────────────────────────────────────

#[tokio::test]
async fn timeout_produces_timeout_error_kind() {
    use axum::extract::Request;

    async fn hanging_handler(_req: Request) -> Response {
        // Sleep longer than the request timeout to force a timeout error.
        tokio::time::sleep(Duration::from_secs(60)).await;
        StatusCode::OK.into_response()
    }

    let app = Router::new().route("/v1/chat/completions", post(hanging_handler));
    let url = spawn_app(app).await;

    // 1-second timeout so the test completes quickly.
    let cfg = RunConfig::build(
        &url,
        "test-model".into(),
        1,
        1,
        false,
        None,
        0,
        None,
        None,
        1,
        None,
        &[],
    )
    .expect("valid config");

    let result = run_grow(&cfg, None, None).await.expect("run completes");
    assert_eq!(result.conversations.len(), 1);
    let conv = &result.conversations[0];
    assert_eq!(conv.turns.len(), 1, "one turn attempted before timeout");
    let turn = &conv.turns[0];
    assert!(!turn.success, "turn must have failed");
    assert_eq!(
        turn.error,
        Some(ErrorKind::Timeout),
        "timeout must be classified as Timeout, not Connect"
    );
}
