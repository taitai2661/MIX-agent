use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct HealthReport {
    pub status: &'static str,
    pub storage: bool,
    pub engine: bool,
    /// True when the process was started with `--safe-mode`. The UI shows a
    /// banner so a user who forgot the flag knows why MCP/browser are missing.
    pub safe_mode: bool,
}

#[tauri::command]
pub async fn health(state: State<'_, AppState>) -> Result<HealthReport, String> {
    let _ = state.storage.pool();
    Ok(HealthReport {
        status: "ok",
        storage: true,
        engine: true,
        safe_mode: crate::safe_mode(),
    })
}
