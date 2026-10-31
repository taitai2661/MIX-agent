//! Bridges the Run Engine's broadcast channel to the Tauri WebView.
//!
//! The engine is a plain tokio broadcast; the WebView expects Tauri events.
//! This bridge is started once from `setup` and forwards every Run event under
//! the `run_event` name with the same JSON shape the Docker edition streams
//! over SSE.

use std::sync::Arc;

use mixd_core::runs::engine::{RunEngine, RunEvent};
use tauri::{AppHandle, Emitter, Runtime};

pub fn spawn<R: Runtime>(app: AppHandle<R>, engine: Arc<RunEngine>) {
    tauri::async_runtime::spawn(async move {
        let mut receiver = engine.subscribe();
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if let Err(err) = app.emit("run_event", &event) {
                        tracing::warn!(?err, "failed to emit run_event to WebView");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "run event receiver lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
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
        let _ = app.emit("run_event", &parsed);
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
