//! Tauri surface for local runtime lifecycle and model jobs.
//!
//! These commands are intentionally thin: the manager owns process ownership
//! and job state, and the UI only asks for a snapshot or an action.

use crate::local_runtime::{JobView, RuntimeSnapshot};
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn local_runtime_snapshot(
    state: State<'_, AppState>,
) -> Result<Vec<RuntimeSnapshot>, String> {
    Ok(state.local_runtime.snapshot().await)
}

#[tauri::command]
pub async fn local_runtime_refresh(
    state: State<'_, AppState>,
) -> Result<Vec<RuntimeSnapshot>, String> {
    state
        .local_runtime
        .refresh()
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn local_runtime_start(
    state: State<'_, AppState>,
    slug: String,
) -> Result<RuntimeSnapshot, String> {
    state
        .local_runtime
        .start(&slug)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn local_runtime_stop(
    state: State<'_, AppState>,
    slug: String,
) -> Result<RuntimeSnapshot, String> {
    state
        .local_runtime
        .stop(&slug)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn local_runtime_restart(
    state: State<'_, AppState>,
    slug: String,
) -> Result<RuntimeSnapshot, String> {
    state
        .local_runtime
        .restart(&slug)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn local_runtime_logs(
    state: State<'_, AppState>,
    slug: String,
) -> Result<Vec<String>, String> {
    Ok(state.local_runtime.logs(&slug).await)
}

#[tauri::command]
pub async fn local_runtime_open_app(
    state: State<'_, AppState>,
    slug: String,
) -> Result<(), String> {
    let target = state
        .local_runtime
        .open_app(&slug)
        .await
        .map_err(crate::error::cmd_err)?;
    open::that(target).map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn local_model_pull(
    state: State<'_, AppState>,
    slug: String,
    model: String,
) -> Result<String, String> {
    state
        .local_runtime
        .pull(&slug, &model)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn local_model_delete(
    state: State<'_, AppState>,
    slug: String,
    model: String,
) -> Result<(), String> {
    state
        .local_runtime
        .delete_model(&slug, &model)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn local_model_load(
    state: State<'_, AppState>,
    slug: String,
    model: String,
) -> Result<(), String> {
    state
        .local_runtime
        .load_model(&slug, &model)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub async fn local_model_unload(
    state: State<'_, AppState>,
    slug: String,
    model: String,
) -> Result<(), String> {
    state
        .local_runtime
        .unload_model(&slug, &model)
        .await
        .map_err(crate::error::cmd_err)
}

#[tauri::command]
pub fn local_job_list(state: State<'_, AppState>) -> Vec<JobView> {
    state.local_runtime.list_jobs()
}

#[tauri::command]
pub fn local_job_cancel(state: State<'_, AppState>, id: String) -> bool {
    state.local_runtime.cancel_job(&id)
}
