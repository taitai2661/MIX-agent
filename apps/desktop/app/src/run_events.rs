//! Bridges the Run Engine's broadcast channel to the Tauri WebView.
//!
//! The engine is a plain tokio broadcast; the WebView expects Tauri events.
//! This bridge is started once from `setup` and forwards every Run event under
//! the `run_event` name with the same JSON shape the Docker edition streams
//! over SSE.

use std::sync::Arc;
use std::time::Duration;

use mixd_core::runs::engine::{RunEngine, RunEvent};
use tauri::{AppHandle, Emitter, Runtime};

use crate::spotlight::SpotlightService;

/// Token deltas are coalesced for this long before crossing the IPC boundary.
/// The UI already batches its own renders, but every `app.emit` is still one
/// WebView message; a fast model used to produce hundreds per second.
const IPC_BATCH_WINDOW: Duration = Duration::from_millis(50);
const IPC_BATCH_MAX: usize = 64;

pub fn spawn<R: Runtime>(
    app: AppHandle<R>,
    engine: Arc<RunEngine>,
    spotlight: Arc<SpotlightService>,
) {
    tauri::async_runtime::spawn(async move {
        let mut receiver = engine.subscribe();
        let mut pending: Vec<RunEvent> = Vec::new();
        loop {
            // Milestone events are flushed immediately; only the token stream
            // waits for the window, so a tool card or status change is never
            // delayed behind buffered text.
            match tokio::time::timeout(IPC_BATCH_WINDOW, receiver.recv()).await {
                Ok(Ok(event)) => {
                    // A finished Run can change the auto-generated title or the
                    // last assistant preview. Queue the conversation here; the
                    // Spotlight work itself stays off this forwarding loop.
                    if event.kind == "status"
                        && event
                            .data
                            .get("status")
                            .and_then(|value| value.as_str())
                            .is_some_and(|status| {
                                matches!(
                                    status,
                                    "completed" | "failed" | "cancelled" | "interrupted"
                                )
                            })
                    {
                        let spotlight = spotlight.clone();
                        let run_id = event.run_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            spotlight.schedule_run(&run_id).await;
                        });
                    }
                    let stream = matches!(event.kind.as_str(), "text" | "reasoning");
                    pending.push(event);
                    if !stream || pending.len() >= IPC_BATCH_MAX {
                        flush(&app, &mut pending);
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped))) => {
                    tracing::warn!(skipped, "run event receiver lagged");
                    flush(&app, &mut pending);
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                    flush(&app, &mut pending);
                    break;
                }
                Err(_) => flush(&app, &mut pending),
            }
        }
    });
}

fn flush<R: Runtime>(app: &AppHandle<R>, pending: &mut Vec<RunEvent>) {
    if pending.is_empty() {
        return;
    }
    let result = if pending.len() == 1 {
        app.emit("run_event", &pending[0])
    } else {
        app.emit("run_event_batch", &*pending)
    };
    if let Err(err) = result {
        tracing::warn!(?err, "failed to emit run events to WebView");
    }
    pending.clear();
}

/// Replay the persisted event log for a Run so a UI that just attached can
/// catch up on what it missed.
pub async fn replay(app: &AppHandle<impl Runtime>, engine: &RunEngine, run_id: &str) {
    // Token deltas are persisted in batches, so a replay that raced the flush
    // window would silently truncate the transcript the UI just asked for.
    engine.flush_all_events().await;
    let limit = mixd_core::runs::events::MAX_REPLAY_EVENTS;
    let window =
        match mixd_core::runs::events::load_replay_window(engine.storage.pool(), run_id, limit)
            .await
        {
            Ok(window) => window,
            Err(err) => {
                tracing::warn!(?err, %run_id, "replay query failed; emitting no events");
                Default::default()
            }
        };
    let omitted = window.omitted(limit);

    let mut last_sequence = 0;
    // Replay a long Run in chunks rather than one WebView message per row.
    // The UI reducer is the same for a batch and a single event.
    const REPLAY_CHUNK: usize = 500;
    let mut batch: Vec<RunEvent> = Vec::with_capacity(REPLAY_CHUNK);
    for (raw_id, sequence, kind, raw_data, raw_created) in window.rows {
        let run_id = match uuid::Uuid::parse_str(&raw_id) {
            Ok(id) => id,
            Err(_) => {
                // Never fabricate a nil UUID: a malformed row would be
                // re-emitted as a different run's event. Skip it loudly.
                tracing::warn!(run_id = %raw_id, "skipping run_event with a malformed run id");
                continue;
            }
        };
        let parsed = RunEvent {
            run_id,
            sequence,
            kind,
            data: serde_json::from_str(&raw_data).unwrap_or_default(),
            created_at: chrono::DateTime::parse_from_rfc3339(&raw_created)
                .map(|t| t.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
        };
        last_sequence = last_sequence.max(parsed.sequence);
        batch.push(parsed);
        if batch.len() >= REPLAY_CHUNK {
            let _ = app.emit("run_event_batch", &batch);
            batch.clear();
        }
    }
    if !batch.is_empty() {
        let _ = app.emit("run_event_batch", &batch);
    }
    if omitted > 0 {
        let marker = RunEvent {
            run_id: uuid::Uuid::parse_str(run_id).unwrap_or_default(),
            sequence: last_sequence + 1,
            kind: "replay_truncated".to_string(),
            data: serde_json::json!({ "limit": limit, "omitted": omitted }),
            created_at: chrono::Utc::now(),
        };
        let _ = app.emit("run_event", &marker);
    }
}
