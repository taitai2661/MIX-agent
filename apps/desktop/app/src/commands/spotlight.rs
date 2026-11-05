//! macOS Spotlight settings and navigation commands.

use crate::spotlight::{SpotlightStatus, SpotlightTarget};
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn spotlight_status(state: State<'_, AppState>) -> Result<SpotlightStatus, String> {
    Ok(state.spotlight.status().await)
}

#[tauri::command]
pub async fn spotlight_rebuild(state: State<'_, AppState>) -> Result<(), String> {
    state
        .spotlight
        .rebuild()
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn spotlight_clear(state: State<'_, AppState>) -> Result<(), String> {
    state.spotlight.clear().await.map_err(crate::error::cmd_err)
}

/// Drain the target from a click that arrived before the WebView listener was
/// ready (cold start from Spotlight).
#[tauri::command]
pub fn spotlight_take_pending(state: State<'_, AppState>) -> Option<SpotlightTarget> {
    state.spotlight.take_pending()
}
