//! OS notifications.
//!
//! The point of these is not decoration: agent runs can take many minutes, so
//! the notification is how the user learns a run finished or needs an approval
//! while they are in another app.

use tauri::{AppHandle, Runtime};
use tauri_plugin_notification::NotificationExt;

/// Events the backend emits that are worth interrupting for.
///
/// The engine publishes exactly one terminal `status` event per run, whose
/// `data.status` is `completed` / `failed` / `cancelled`. There is no
/// `run_completed` / `run_failed` kind — matching on those names is why no run
/// ever produced a notification.
pub const EVT_STATUS: &str = "status";
pub const EVT_APPROVAL_REQUESTED: &str = "tool_approval_requested";

/// Ask the OS to show a notification. Failures are logged, never fatal: a
/// missing notification must not take down a finished run.
pub fn notify<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str) {
    if let Err(err) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(%err, title, "could not show a notification");
    }
}

/// Ask the OS for permission. Safe to call more than once.
pub fn request_permission<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri_plugin_notification::NotificationExt;
        if let Err(err) = handle.notification().permission_state() {
            tracing::warn!(%err, "could not read the notification permission state");
        }
    });
}

/// Watch Run Engine events and surface the two that matter.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    use tauri::Manager;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let engine = handle.state::<crate::AppState>().engine.clone();
        let mut receiver = engine.subscribe();
        loop {
            let event = match receiver.recv().await {
                Ok(event) => event,
                // A receiver that falls behind must not lose notifications for
                // the rest of the session: the engine's buffer is finite, so a
                // burst of token events can lag this watcher. The WebView bridge
                // already treats `Lagged` as "skip and keep going"; this watcher
                // used to exit its loop instead.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "notification watcher lagged; continuing");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if event.kind == EVT_APPROVAL_REQUESTED {
                let tool = event
                    .data
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("a tool");
                notify(
                    &handle,
                    "MIX-agent — approval needed",
                    &format!("Allow the agent to run `{tool}`?"),
                );
                continue;
            }
            if event.kind != EVT_STATUS {
                continue;
            }
            match event.data.get("status").and_then(|v| v.as_str()) {
                Some("completed") => {
                    let summary = event
                        .data
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Run finished");
                    notify(&handle, "MIX-agent — run finished", &truncate(summary, 160));
                }
                Some("failed") => {
                    let err = event
                        .data
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Run failed");
                    notify(&handle, "MIX-agent — run failed", &truncate(err, 160));
                }
                _ => {}
            }
        }
    });
}

/// Keep notifications to one readable line.
fn truncate(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn collapses_whitespace() {
        assert_eq!(truncate("a\n\n  b\tc", 100), "a b c");
    }

    #[test]
    fn short_text_is_untouched() {
        assert_eq!(truncate("done", 100), "done");
    }

    #[test]
    fn long_text_is_truncated_with_an_ellipsis() {
        let out = truncate(&"x".repeat(50), 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncation_does_not_split_a_char() {
        let out = truncate(&"日本語テキスト".repeat(10), 5);
        assert!(out.ends_with('…'));
        // Valid UTF-8 with no replacement character.
        assert!(!out.contains('\u{FFFD}'));
    }
}
