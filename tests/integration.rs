//! End-to-end integration tests driving the real `client`/`runner` against
//! local mock servers (§12). wiremock covers the non-streaming case; a tiny
//! axum server provides chunked SSE with a final usage chunk.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::post;
use futures_util::StreamExt;
use futures_util::stream;

use llmprobe::config::RunConfig;
use llmprobe::metrics::aggregate;
use llmprobe::report;
use llmprobe::runner::{BatchResult, run_batch};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a `RunConfig` against `base_url` with sensible test defaults.
fn cfg(base_url: &str, requests: usize, concurrency: usize, stream: bool) -> RunConfig {
    RunConfig::build(
        base_url,
        "test-model".to_string(),
        requests,
        concurrency,
        stream,
        "hello".to_string(),
        64,
        None,
        10,
        0,
        None,
        &[],
    )
    .expect("valid config")
}

#[tokio::test]
async fn non_streaming_round_trip_via_wiremock() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "choices": [{ "message": { "content": "hi there" } }],
        "usage": { "prompt_tokens": 5, "completion_tokens": 12 }
    });
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let cfg = cfg(&server.uri(), 5, 1, false);
    let BatchResult {
        outcomes,
        wall_clock,
    } = run_batch(&cfg, None, None).await.expect("batch runs");
    let report = aggregate(&outcomes, wall_clock);

    assert_eq!(report.total, 5);
    assert_eq!(report.ok, 5);
    assert_eq!(report.failed, 0);
    assert_eq!(report.total_completion_tokens, 5 * 12);
    assert!(report.e2e.is_some(), "e2e stats present");
    assert!(report.ttft.is_none(), "no TTFT for non-streaming");
    assert!(report.avg_tps.is_some(), "non-streaming TPS uses e2e");

    // --json must be valid and carry the A.11 keys.
    let json = report::render_json(&report, &cfg).expect("json renders");
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(v["summary"]["ok"], 5);
    assert!(v["ttft_s"].is_null());
    assert_eq!(v["throughput"]["completion_tokens_total"], 60);
    assert!(v["latency_e2e_s"]["p95"].is_number());
}

#[tokio::test]
async fn reports_http_errors_without_aborting() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
        .mount(&server)
        .await;

    let cfg = cfg(&server.uri(), 4, 2, false);
    let BatchResult {
        outcomes,
        wall_clock,
    } = run_batch(&cfg, None, None).await.expect("batch runs");
    let report = aggregate(&outcomes, wall_clock);

    assert_eq!(report.ok, 0);
    assert_eq!(report.failed, 4);
    // All failures classify as HTTP 429 — a single breakdown bucket.
    assert_eq!(report.error_breakdown.len(), 1);
    assert_eq!(report.error_breakdown[0].1, 4);
}

/// SSE handler: a role-priming chunk, several delayed content tokens, a final
/// usage chunk with **empty** `choices`, then `[DONE]`.
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

    // Stagger frames so the decode window (gen_time) is non-zero and TPS/ITL
    // are deterministically computable.
    let body = stream::iter(frames).then(|f| async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok::<Bytes, Infallible>(Bytes::from(f))
    });
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(body),
    )
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

#[tokio::test]
async fn streaming_populates_ttft_and_tps_via_axum() {
    let url = spawn_app(Router::new().route("/v1/chat/completions", post(sse_handler))).await;

    let cfg = cfg(&url, 3, 1, true);
    let BatchResult {
        outcomes,
        wall_clock,
    } = run_batch(&cfg, None, None).await.expect("batch runs");
    let report = aggregate(&outcomes, wall_clock);

    assert_eq!(report.ok, 3, "all streaming requests succeed");
    assert_eq!(
        report.total_completion_tokens,
        3 * 8,
        "usage captured from the empty-choices final chunk"
    );
    assert!(report.ttft.is_some(), "TTFT present when streaming");
    assert!(report.avg_tps.is_some(), "decode TPS computed");
    assert!(report.mean_itl_ms.is_some(), "inter-token latency computed");

    // Every request individually carries TTFT and a token count.
    for o in &outcomes {
        assert!(o.success);
        assert!(o.ttft.is_some());
        assert_eq!(o.completion_tokens, Some(8));
    }
}

/// Delayed JSON handler tracking peak concurrency via shared atomics.
async fn counted_handler(
    axum::extract::State(counters): axum::extract::State<Arc<(AtomicUsize, AtomicUsize)>>,
) -> impl IntoResponse {
    let (cur, max) = (&counters.0, &counters.1);
    let now = cur.fetch_add(1, Ordering::SeqCst) + 1;
    max.fetch_max(now, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(50)).await;
    cur.fetch_sub(1, Ordering::SeqCst);
    axum::Json(serde_json::json!({
        "choices": [{ "message": { "content": "x" } }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1 }
    }))
}

#[tokio::test]
async fn concurrency_is_bounded_by_config() {
    let counters = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
    let app = Router::new()
        .route("/v1/chat/completions", post(counted_handler))
        .with_state(counters.clone());
    let url = spawn_app(app).await;

    let cfg = cfg(&url, 12, 4, false);
    let result = run_batch(&cfg, None, None).await.expect("batch runs");
    let report = aggregate(&result.outcomes, result.wall_clock);

    assert_eq!(report.ok, 12);
    let peak = counters.1.load(Ordering::SeqCst);
    assert!(peak <= 4, "never more than C=4 in flight, saw {peak}");
    assert!(peak >= 2, "concurrency actually used, saw {peak}");
}
