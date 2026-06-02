//! Orchestrate a batch of requests and emit live events.
//!
//! Phase 1 runs sequentially; Phase 3 swaps the loop for bounded concurrency
//! (`buffer_unordered`) and adds Ctrl-C cancellation. The same `run_batch`
//! feeds both the plain report and the TUI — only `tx` differs.

use crate::client::{build_client, run_one};
use crate::config::RunConfig;
use crate::error::ProbeError;
use crate::metrics::RequestOutcome;
use futures_util::{StreamExt, stream};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;

/// Live progress events for the TUI (A.10). `Finished` carries the full outcome
/// (its `success` flag distinguishes ok from failed).
#[derive(Debug, Clone)]
pub enum RunEvent {
    Started { id: usize },
    FirstToken { id: usize, ttft: Duration },
    Finished { id: usize, outcome: RequestOutcome },
}

/// Outcomes plus the wall-clock window they span.
///
/// `wall_clock` is measured around the *measured* batch only — warmup requests
/// run first and are excluded — so `req_per_s` reflects real throughput.
pub struct BatchResult {
    pub outcomes: Vec<RequestOutcome>,
    pub wall_clock: Duration,
}

/// When running indefinitely (`requests == 0`), retain at most this many recent
/// outcomes so memory stays bounded; the report then reflects the latest window.
const INFINITE_KEEP: usize = 5_000;

/// Run `cfg.warmup` discarded requests, then the measured batch with at most
/// `cfg.concurrency` requests in flight (§7). `Ctrl-C` cancels the batch and
/// returns whatever completed so a partial report can still render.
///
/// `cfg.requests == 0` runs indefinitely (until `Ctrl-C`), keeping a rolling
/// window of the most recent outcomes.
///
/// `pause`, when supplied, gates dispatch: while it holds `true`, no new request
/// is sent (in-flight ones still finish), so the caller can pause the load.
pub async fn run_batch(
    cfg: &RunConfig,
    tx: Option<UnboundedSender<RunEvent>>,
    pause: Option<watch::Receiver<bool>>,
) -> Result<BatchResult, ProbeError> {
    let client = build_client(cfg)?;

    // Warmup pre-pass: results discarded, no events emitted.
    for id in 0..cfg.warmup {
        let _ = run_one(&client, cfg, id, None).await;
    }

    let unbounded = cfg.requests == 0;
    let count = if unbounded { usize::MAX } else { cfg.requests };

    let batch_start = Instant::now();
    let mut in_flight = stream::iter(0..count)
        .map(|id| {
            let client = &client;
            let tx = tx.clone();
            let pause = pause.clone();
            async move {
                // Park here while paused (before sending), so toggling pause
                // stops new requests while in-flight ones drain.
                if let Some(mut pr) = pause {
                    while *pr.borrow() {
                        if pr.changed().await.is_err() {
                            break;
                        }
                    }
                }
                let outcome = run_one(client, cfg, id, tx.as_ref()).await;
                if let Some(tx) = tx.as_ref() {
                    let _ = tx.send(RunEvent::Finished {
                        id,
                        outcome: outcome.clone(),
                    });
                }
                outcome
            }
        })
        .buffer_unordered(cfg.concurrency);

    let mut outcomes: VecDeque<RequestOutcome> = VecDeque::new();
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            next = in_flight.next() => match next {
                Some(outcome) => {
                    outcomes.push_back(outcome);
                    if unbounded && outcomes.len() > INFINITE_KEEP {
                        outcomes.pop_front();
                    }
                }
                None => break,
            },
            _ = &mut ctrl_c => break, // cancel: drop in-flight, keep partial results
        }
    }
    let wall_clock = batch_start.elapsed();

    Ok(BatchResult {
        outcomes: outcomes.into(),
        wall_clock,
    })
}
