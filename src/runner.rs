//! Conversation grow runner.
//!
//! Spawns `cfg.concurrency` independent slot tasks. Each slot loops over
//! conversations: it grows a conversation turn-by-turn until the server
//! refuses with a context-overflow error (or until `--max-turns` is hit),
//! records the `ConversationOutcome`, then starts the next conversation.
//! A shared atomic counter gates `-n` (total conversations); `0` means run
//! indefinitely.

use crate::client::{build_client, send_turn};
use crate::config::RunConfig;
use crate::prompts::PromptSampler;
use crate::error::ProbeError;
use crate::metrics::{ConfigSnapshot, ConversationOutcome, GrowResult, TerminalReason, TurnOutcome};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;

/// Live progress events streamed to the TUI.
#[derive(Debug, Clone)]
pub enum RunEvent {
    ConvStarted {
        conv_id: usize,
        slot: usize,
    },
    TurnStarted {
        conv_id: usize,
        turn_idx: usize,
    },
    TurnFirstToken {
        conv_id: usize,
        turn_idx: usize,
        ttft: Duration,
    },
    TurnFinished {
        conv_id: usize,
        turn_idx: usize,
        outcome: TurnOutcome,
    },
    ConvFinished {
        conv_id: usize,
        outcome: ConversationOutcome,
    },
}

/// Run the grow loop and return all completed conversations plus wall-clock.
pub async fn run_grow(
    cfg: &RunConfig,
    tx: Option<UnboundedSender<RunEvent>>,
    pause: Option<watch::Receiver<bool>>,
) -> Result<GrowResult, ProbeError> {
    let client = Arc::new(build_client(cfg)?);

    let total = cfg.conversations;
    let completed = Arc::new(AtomicUsize::new(0));
    let next_conv_id = Arc::new(AtomicUsize::new(0));

    // Collected conversation outcomes, guarded by a mutex so slot tasks can push.
    let outcomes: Arc<tokio::sync::Mutex<Vec<ConversationOutcome>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let batch_start = Instant::now();

    let mut handles = Vec::new();
    for slot in 0..cfg.concurrency {
        let client = client.clone();
        let cfg = cfg.clone();
        let tx = tx.clone();
        let pause = pause.clone();
        let completed = completed.clone();
        let next_conv_id = next_conv_id.clone();
        let outcomes = outcomes.clone();

        handles.push(tokio::spawn(async move {
            slot_loop(
                slot, &client, &cfg, tx.as_ref(), pause.as_ref(),
                &completed, &next_conv_id, total, &outcomes,
            )
            .await;
        }));
    }

    // Await Ctrl-C or all slots finishing.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    tokio::select! {
        _ = futures_util::future::join_all(handles) => {}
        _ = &mut ctrl_c => {
            // Mark any in-progress conversations as cancelled by letting them
            // drain naturally — slots check `completed` each iteration.
            // For now we just take what we have.
        }
    }

    let wall_clock = batch_start.elapsed();
    // All slot tasks have joined by this point, so Arc is uniquely owned.
    let conversations = match Arc::try_unwrap(outcomes) {
        Ok(m) => m.into_inner(),
        Err(_) => unreachable!("all slot tasks completed before unwrap"),
    };

    Ok(GrowResult {
        conversations,
        wall_clock,
        config: ConfigSnapshot {
            endpoint: cfg.endpoint.clone(),
            model: cfg.model.clone(),
            concurrency: cfg.concurrency,
            stream: cfg.stream,
            max_tokens: cfg.max_tokens,
        },
    })
}

async fn slot_loop(
    slot: usize,
    client: &reqwest::Client,
    cfg: &RunConfig,
    tx: Option<&UnboundedSender<RunEvent>>,
    pause: Option<&watch::Receiver<bool>>,
    completed: &AtomicUsize,
    next_conv_id: &AtomicUsize,
    total: usize,
    outcomes: &tokio::sync::Mutex<Vec<ConversationOutcome>>,
) {
    loop {
        // Check if we've hit the conversation target.
        if total > 0 && completed.load(Ordering::Relaxed) >= total {
            break;
        }

        let conv_id = next_conv_id.fetch_add(1, Ordering::Relaxed);

        // Re-check after claiming the id (avoids racing past the limit).
        if total > 0 && conv_id >= total {
            break;
        }

        if let Some(tx) = tx {
            let _ = tx.send(RunEvent::ConvStarted { conv_id, slot });
        }

        let conv = run_conversation(slot, conv_id, client, cfg, tx, pause).await;

        if let Some(tx) = tx {
            let _ = tx.send(RunEvent::ConvFinished {
                conv_id,
                outcome: conv.clone(),
            });
        }

        outcomes.lock().await.push(conv);
        completed.fetch_add(1, Ordering::Relaxed);
    }
}

async fn run_conversation(
    slot: usize,
    conv_id: usize,
    client: &reqwest::Client,
    cfg: &RunConfig,
    tx: Option<&UnboundedSender<RunEvent>>,
    pause: Option<&watch::Receiver<bool>>,
) -> ConversationOutcome {
    let conv_start = Instant::now();

    let mut sampler = PromptSampler::new(cfg.rng_seed);

    // System prompt: fixed override or random pool sample.
    let system = cfg.system_prompt.as_deref().unwrap_or_else(|| sampler.system());

    // Build initial messages: optional system turn + pool seed.
    let mut messages: Vec<(String, String)> = {
        let mut m = Vec::new();
        if !system.is_empty() {
            m.push(("system".into(), system.to_owned()));
        }
        m.push(("user".into(), sampler.seed().to_owned()));
        m
    };

    let mut turns: Vec<TurnOutcome> = Vec::new();

    let terminal = 'conv: loop {
        // Honour pause before each turn dispatch.
        if let Some(pr) = pause {
            let mut rx = pr.clone();
            while *rx.borrow() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }

        let turn_idx = turns.len();

        // Check max-turns cap.
        if cfg.max_turns_per_conv > 0 && turn_idx >= cfg.max_turns_per_conv {
            break 'conv TerminalReason::MaxTurns;
        }

        let outcome = send_turn(client, cfg, conv_id, turn_idx, &messages, tx).await;

        if let Some(tx) = tx {
            let _ = tx.send(RunEvent::TurnFinished {
                conv_id,
                turn_idx,
                outcome: outcome.clone(),
            });
        }

        let success = outcome.success;
        let error = outcome.error;
        let reply = outcome.reply.clone();

        turns.push(outcome);

        if !success {
            use crate::metrics::ErrorKind;
            break 'conv match error {
                Some(ErrorKind::ContextOverflow) => TerminalReason::ContextLimit,
                Some(k) => TerminalReason::Error(k),
                None => TerminalReason::Error(ErrorKind::Connect),
            };
        }

        // Append assistant reply and follow-up user turn to grow the conversation.
        let assistant_text = reply.unwrap_or_default();
        messages.push(("assistant".into(), assistant_text));
        messages.push(("user".into(), sampler.next().to_owned()));
    };

    ConversationOutcome {
        id: conv_id,
        slot,
        turns,
        terminal,
        wall_clock: conv_start.elapsed(),
    }
}
