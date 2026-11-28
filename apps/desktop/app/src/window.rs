//! Extra application windows.
//!
//! A coding agent often has two jobs in flight: one Run writing code, another
//! reviewing a diff or a second repository. The single-window shell forced the
//! user to close and reopen threads to compare them. Each window gets its own
//! WebView (and therefore its own UI state); the Rust backend is shared, so
//! Runs, approvals and the database are the same.

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Runtime, WebviewUrl, WebviewWindowBuilder};

static NEXT_LABEL: AtomicU64 = AtomicU64::new(1);

/// Open another main-window-shaped WebView. The label is unique so Tauri does
/// not return the existing window instead of creating one.
pub fn open<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<tauri::WebviewWindow<R>> {
    let label = format!("window-{}", NEXT_LABEL.fetch_add(1, Ordering::Relaxed));
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("MIX-agent Desktop")
        .inner_size(1200.0, 800.0)
        .min_inner_size(880.0, 560.0)
        .resizable(true)
        .center()
        .build()
}
