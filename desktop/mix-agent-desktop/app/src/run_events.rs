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
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT run_id, sequence, kind, data, created_at FROM run_events \
         WHERE run_id = ? ORDER BY sequence ASC",
    )
    .bind(run_id)
    .fetch_all(engine.storage.pool())
    .await
    .unwrap_or_default();

    for row in rows {
        let raw_id: String = row.try_get("run_id").unwrap_or_default();
        let data: serde_json::Value = row
            .try_get::<String, _>("data")
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let parsed = RunEvent {
            run_id: uuid::Uuid::parse_str(&raw_id).unwrap_or_default(),
            sequence: row.try_get::<i64, _>("sequence").unwrap_or_default(),
            kind: row.try_get::<String, _>("kind").unwrap_or_default(),
            data,
            created_at: row
                .try_get::<String, _>("created_at")
                .ok()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|t| t.with_timezone(&chrono::Utc))
                .unwrap_or_else(chrono::Utc::now),
        };
        let _ = app.emit("run_event", &parsed);
    }
}
