use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct HealthReport {
    pub status: &'static str,
    pub storage: bool,
    pub engine: bool,
}

#[tauri::command]
pub async fn health(state: State<'_, AppState>) -> Result<HealthReport, String> {
    let _ = state.storage.pool();
    Ok(HealthReport {
        status: "ok",
        storage: true,
        engine: true,
    })
}
