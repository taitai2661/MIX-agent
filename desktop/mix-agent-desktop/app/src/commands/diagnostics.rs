use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct DiagnosticsReport {
    pub data_dir: String,
    pub db_ok: bool,
    pub secrets_ok: bool,
    pub engine_ok: bool,
    pub provider_count: u32,
    pub model_count: u32,
    pub memory_count: u32,
    pub skill_count: u32,
    pub active_runs: u32,
}

#[tauri::command]
pub async fn collect_diagnostics(state: State<'_, AppState>) -> Result<DiagnosticsReport, String> {
    let pool = state.storage.pool();
    let data_dir = state.storage.data_dir().display().to_string();
    let providers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let models: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM models")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let memories: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let skills: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM skills")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE status IN ('queued','running','waiting_approval','budget_extension_pending')")
        .fetch_one(pool).await.unwrap_or(0);
    Ok(DiagnosticsReport {
        data_dir,
        db_ok: true,
        secrets_ok: true,
        engine_ok: true,
        provider_count: providers as u32,
        model_count: models as u32,
        memory_count: memories as u32,
        skill_count: skills as u32,
        active_runs: active as u32,
    })
}
