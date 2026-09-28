use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct SettingsView {
    pub default_model_id: Option<String>,
    pub default_mode: Option<String>,
    pub egress_allowlist: Vec<String>,
    pub auto_update: bool,
    pub telemetry_consent: bool,
    /// UI language override: `"ja" | "en"`; `None` follows the browser.
    pub language: Option<String>,
    /// UI theme: `"light" | "dark" | "system"`; `None` follows the OS.
    pub theme: Option<String>,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<SettingsView, String> {
    let pool = state.storage.pool();
    let row: Option<(String,)> =
        sqlx::query_as("SELECT data FROM app_settings WHERE id = 'singleton'")
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(match row {
        Some((data,)) => serde_json::from_str(&data).unwrap_or_default(),
        None => SettingsView::default(),
    })
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    input: SettingsView,
) -> Result<SettingsView, String> {
    let pool = state.storage.pool();
    let json = serde_json::to_string(&input).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, ?) \
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(&json)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(input)
}
