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
    /// `Some(true)` when the Keychain holds a usable DB master key,
    /// `Some(false)` when it does not (one will be minted at first use),
    /// `None` when the Keychain refused to answer.
    pub db_key_present: Option<bool>,
    /// True when the database itself is encrypted at rest. Today the storage
    /// layer runs against a vanilla SQLite, so this is always `false`; the
    /// field is wired in so the UI can show a single line that will flip to
    /// `true` once the SQLCipher migration lands.
    pub db_encrypted: bool,
}

#[tauri::command]
pub async fn collect_diagnostics(state: State<'_, AppState>) -> Result<DiagnosticsReport, String> {
    let pool = state.storage.pool();
    let data_dir = state.storage.data_dir().display().to_string();
    // These three used to be literal `true`, which made the pane report a
    // healthy app even when the database, the Keychain or the tool registry
    // were unavailable — exactly the situation the pane exists for.
    let db_ok = match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(pool)
        .await
    {
        Ok(_) => true,
        Err(err) => {
            tracing::error!(%err, "diagnostics: database probe failed");
            false
        }
    };
    let secrets_ok = match state.secrets.probe().await {
        Ok(()) => true,
        Err(err) => {
            tracing::error!(%err, "diagnostics: Keychain probe failed");
            false
        }
    };
    // The registry always answers; an empty list means the built-in tools
    // never loaded, which is what a "run works but calls nothing" report looks
    // like from the outside.
    let tool_count = state.engine.tool_specs().await.len();
    let engine_ok = tool_count > 0;
    if !engine_ok {
        tracing::error!("diagnostics: tool registry advertises no tools");
    }
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
    let db_key_present = state.secrets.db_key_status().await;
    let db_encrypted = crate::commands::settings::read_settings(pool)
        .await
        .map(|s| s.database_encryption)
        .unwrap_or(false);
    Ok(DiagnosticsReport {
        data_dir,
        db_ok,
        secrets_ok,
        engine_ok,
        provider_count: providers as u32,
        model_count: models as u32,
        memory_count: memories as u32,
        skill_count: skills as u32,
        active_runs: active as u32,
        db_key_present,
        db_encrypted,
    })
}

#[derive(Serialize)]
pub struct DiagnosticsExport {
    pub report: DiagnosticsReport,
    pub os: String,
    pub arch: String,
    pub app_version: String,
    pub recent_logs: String,
}

#[tauri::command]
pub async fn get_recent_logs(lines: Option<usize>) -> Result<String, String> {
    let limit = lines.unwrap_or(200).min(2000);
    let Some(dir) = dirs::data_dir().map(|base| base.join(crate::APP_IDENTIFIER)) else {
        return Ok("log directory not found".to_string());
    };
    if !dir.is_dir() {
        return Ok("no log directory exists yet".to_string());
    }

    // Find the latest mix-agent.log.* file or mix-agent.log
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with("mix-agent.log") {
                        if let Ok(meta) = entry.metadata() {
                            if let Ok(modified) = meta.modified() {
                                files.push((modified, path));
                            }
                        }
                    }
                }
            }
        }
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let Some((_, latest_path)) = files.first() else {
        return Ok("no log file found".to_string());
    };

    match tokio::fs::read_to_string(latest_path).await {
        Ok(content) => {
            let all_lines: Vec<&str> = content.lines().collect();
            let start = all_lines.len().saturating_sub(limit);
            Ok(all_lines[start..].join("\n"))
        }
        Err(e) => Ok(format!("could not read log file: {e}")),
    }
}

#[tauri::command]
pub async fn export_diagnostics(
    state: State<'_, AppState>,
) -> Result<DiagnosticsExport, String> {
    let report = collect_diagnostics(state).await?;
    let recent_logs = get_recent_logs(Some(300)).await.unwrap_or_default();
    Ok(DiagnosticsExport {
        report,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        recent_logs,
    })
}

