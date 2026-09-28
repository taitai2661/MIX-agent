use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Deserialize)]
pub struct BackupCreateInput {
    /// Absolute path to write the `.mixbak` file to.
    pub destination: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct BackupCreated {
    pub path: String,
    pub bytes: u64,
    pub created_at: String,
    pub tables: usize,
    pub needs_restart: bool,
}

#[tauri::command]
pub async fn create_backup(
    state: State<'_, AppState>,
    input: BackupCreateInput,
) -> Result<BackupCreated, String> {
    let info = mixd_storage::backup::create(
        state.storage.data_dir(),
        &input.destination,
        &input.password,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(BackupCreated {
        path: info.path,
        bytes: info.bytes,
        created_at: info.manifest.created_at,
        tables: info.manifest.tables.len(),
        needs_restart: false,
    })
}

#[derive(Deserialize)]
pub struct BackupRestoreInput {
    /// Absolute path of the `.mixbak` file to read.
    pub source: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct BackupRestored {
    pub staged_path: String,
    pub created_at: String,
    pub tables: usize,
    /// The running process holds an open SQLite pool, so the restored snapshot
    /// is promoted on the next launch. Callers should relaunch.
    pub needs_restart: bool,
}

#[tauri::command]
pub async fn restore_backup(
    state: State<'_, AppState>,
    input: BackupRestoreInput,
) -> Result<BackupRestored, String> {
    let info =
        mixd_storage::backup::restore(state.storage.data_dir(), &input.source, &input.password)
            .await
            .map_err(|e| e.to_string())?;
    Ok(BackupRestored {
        staged_path: info.staged_path,
        created_at: info.manifest.created_at,
        tables: info.manifest.tables.len(),
        needs_restart: true,
    })
}

/// Relaunch after a restore so the staged snapshot is promoted.
#[tauri::command]
pub async fn relaunch_app(app: tauri::AppHandle) -> Result<(), String> {
    app.restart();
}
