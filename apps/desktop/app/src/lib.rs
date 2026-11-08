//! MIX-agent Desktop application library.

pub mod autostart;
pub mod cli;
pub mod commands;
pub mod deep_link;
pub mod diff;
pub mod embedding;
pub mod error;
pub mod http;
pub mod local_runtime;
pub mod menu;
pub mod path_grants;
pub mod push;
pub mod run_events;
pub mod scheduler;
pub mod spotlight;
pub mod tray;
pub mod webview_browser;
pub mod workspace_baseline;
pub mod worktree;

use mixd_browser::BrowserHost as _;
use mixd_core::runs::engine::{ProviderRegistry, RunEngine};
use mixd_mcp::McpHost;
use mixd_providers::{
    anthropic::AnthropicProvider, chatgpt::ChatGptProvider, gemini::GeminiProvider,
    openai::OpenAiProvider, preset as preset_catalog, Provider,
};
use mixd_storage::{SecretStore, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};

/// Emitted when the user asked to quit while Runs are still active. The
/// WebView owns the confirmation dialog; Rust must not silently kill a live
/// coding task (or a scheduled job) just because the menu item was clicked.
pub const EVT_QUIT_BLOCKED: &str = "app://quit-blocked";

/// Set only by an explicit "stop the runs and quit" decision (or a restart
/// requested by the updater/relaunch path). Without it every `app.exit`
/// would re-enter the same guard.
static ALLOW_EXIT: AtomicBool = AtomicBool::new(false);

async fn count_active_runs(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM runs WHERE status IN \
         ('queued','running','waiting_approval','budget_extension_pending')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// Let the next `ExitRequested` pass through the guard.
///
/// The command that stops live Runs calls this immediately before `exit(0)`.
pub fn allow_exit() {
    ALLOW_EXIT.store(true, Ordering::SeqCst);
}

/// Ask the app to quit, but give the user a chance to see what would be lost.
///
/// Called by the tray (and any future native menu item that bypasses
/// `ExitRequested`). The menu's predefined Quit still goes through the run
/// loop, which applies the same guard.
pub fn request_quit<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let active = app
        .try_state::<AppState>()
        .map(|state| tauri::async_runtime::block_on(count_active_runs(state.storage.pool())))
        .unwrap_or(0);
    if active > 0 && !ALLOW_EXIT.load(Ordering::SeqCst) {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        let _ = app.emit(EVT_QUIT_BLOCKED, serde_json::json!({ "count": active }));
        return;
    }
    ALLOW_EXIT.store(true, Ordering::SeqCst);
    app.exit(0);
}

mod adapters;
mod app_run;
mod providers;
mod startup;
mod state;
#[cfg(test)]
mod tests;

/// Tiny local re-export so the bridges can name the trait without another
/// workspace dependency.
mod async_trait_impl {
    pub use async_trait::async_trait;
}

pub use worktree::cleanup_stale_worktrees;
use worktree::{cleanup_delegate_worktree, collect_worktree_diff, prepare_delegate_worktree};

pub(crate) use adapters::*;
pub(crate) use providers::*;
pub(crate) use startup::*;
pub(crate) use state::{parent_model, refresh_registry};

pub use app_run::run;
pub use providers::{build_provider, register_adapter, resolve_provider_for_model};
pub use startup::APP_IDENTIFIER;
pub use state::{build_app_state, AppState};
