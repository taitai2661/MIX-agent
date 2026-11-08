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
    collect_diagnostics_for(state.inner()).await
}

pub async fn collect_diagnostics_for(state: &AppState) -> Result<DiagnosticsReport, String> {
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
    // The SQLCipher passphrase is not wired into the connection yet, so the
    // database is plaintext regardless of what an older settings row says.
    // Report the actual state instead of echoing the stored flag.
    let db_encrypted = false;
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

    // Directory scans and file reads are synchronous. Running them on the
    // blocking pool keeps a large/cold log file from stalling the Tauri async
    // runtime (and every other command queued behind it).
    tokio::task::spawn_blocking(move || read_recent_logs(&dir, limit))
        .await
        .map_err(crate::error::cmd_err)
}

/// Read at most `limit` trailing lines, and at most `MAX_LOG_READ_BYTES` from
/// the end of the file: a multi-gigabyte log is not loaded just to show 200
/// lines.
fn read_recent_logs(dir: &std::path::Path, limit: usize) -> String {
    use std::io::{Read, Seek, SeekFrom};

    const MAX_LOG_READ_BYTES: u64 = 2 * 1024 * 1024;

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
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
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    let Some((_, latest_path)) = files.first() else {
        return "no log file found".to_string();
    };

    let mut file = match std::fs::File::open(latest_path) {
        Ok(file) => file,
        Err(e) => return format!("could not read log file: {e}"),
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(MAX_LOG_READ_BYTES);
    if start > 0 {
        if let Err(e) = file.seek(SeekFrom::Start(start)) {
            return format!("could not read log file: {e}");
        }
    }
    let mut bytes = Vec::new();
    if let Err(e) = file.read_to_end(&mut bytes) {
        return format!("could not read log file: {e}");
    }
    let content = String::from_utf8_lossy(&bytes);
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(limit);
    all_lines[start..].join("\n")
}

#[tauri::command]
pub async fn export_diagnostics(state: State<'_, AppState>) -> Result<DiagnosticsExport, String> {
    export_diagnostics_for(state.inner()).await
}

pub async fn export_diagnostics_for(state: &AppState) -> Result<DiagnosticsExport, String> {
    let report = collect_diagnostics_for(state).await?;
    let recent_logs = get_recent_logs(Some(300)).await.unwrap_or_default();
    Ok(DiagnosticsExport {
        report,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        recent_logs,
    })
}

/// A single zip a support request can attach: the JSON report, the log tail,
/// and a short note explaining what is (and is not) inside.
///
/// The JSON export already existed, but asking a user to find the log file and
/// attach two downloads is friction at exactly the moment they are frustrated.
/// Provider keys live in the OS secret store and are not included; the settings
/// document is omitted entirely rather than relying on a redactor to catch a
/// future field.
#[tauri::command]
pub async fn diagnostics_bundle(state: State<'_, AppState>) -> Result<String, String> {
    use base64::Engine as _;

    let exported = export_diagnostics_for(state.inner()).await?;
    let report = serde_json::to_vec_pretty(&exported)
        .map_err(|err| format!("could not serialise diagnostics: {err}"))?;
    let readme = format!(
        "MIX-agent Desktop diagnostics bundle\n\
         Version: {}\nOS: {} ({})\n\n\
         Contents:\n\
         - diagnostics.json  machine-readable report and 300-line log tail\n\
         - recent.log        the same log tail as a plain text file\n\
         - schema.txt        SQLite schema version\n\n\
         Secrets (provider API keys, OAuth tokens) are stored in the OS \
         credential store and are deliberately absent. Settings are not \
         included. Review recent.log before sharing if your workspace paths \
         are sensitive.\n",
        exported.app_version, exported.os, exported.arch
    );
    let logs = exported.recent_logs.clone().into_bytes();
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        ("diagnostics.json".to_string(), report),
        ("recent.log".to_string(), logs),
        ("README.txt".to_string(), readme.into_bytes()),
    ];
    if let Ok(version) = sqlx::query_scalar::<_, i64>("PRAGMA user_version")
        .fetch_one(state.storage.pool())
        .await
    {
        entries.push((
            "schema.txt".to_string(),
            format!("user_version={version}\n").into_bytes(),
        ));
    }
    let bytes = crate::commands::zip_util::write_zip(&entries)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}
