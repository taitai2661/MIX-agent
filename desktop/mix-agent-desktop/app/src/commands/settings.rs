use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

/// The single settings document.
///
/// `#[serde(default)]` sits on the container rather than on individual fields
/// so one rule covers the whole struct: any key missing from the stored JSON
/// falls back to `Default` instead of failing the parse. Without it a
/// hand-edited or truncated row would deserialize to an error and
/// [`read_settings`] would throw away *every* stored preference, not just the
/// unreadable one. It also means a field added in a future version reads back
/// as its default rather than blanking the document written by an older build.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
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
    /// Default parent folder for auto-generated project workspaces
    /// (Projectless chats, legacy backfills). `None` →
    /// `~/Documents/MIX-agent Projects`.
    pub projects_root: Option<String>,
    /// Codex-compatible `project_doc_max_bytes`: combined byte cap for the
    /// `AGENTS.md` chain. `None` → 32 KiB.
    pub project_doc_max_bytes: Option<u64>,
    /// Codex-compatible `project_doc_fallback_filenames`: extra instruction
    /// filenames consulted after `AGENTS.override.md` / `AGENTS.md`.
    pub project_doc_fallback_filenames: Vec<String>,
    /// App-wide default for the project's own check, used by any project that
    /// does not override it. `None` leaves the verification gate off.
    pub verify_command: Option<String>,
    /// App-wide default formatter/lint command reported to the model.
    pub format_command: Option<String>,
    /// Refuse shell commands that write outside the workspace, and any
    /// privilege-escalation wrapper, instead of merely flagging them.
    pub sandbox_terminal: bool,
    /// Whether the first-run wizard has been finished or explicitly deferred.
    ///
    /// `None` is the same as `false` to the UI: a fresh install has never
    /// written this key, and the wizard must appear. It is persisted rather
    /// than kept in `localStorage` so that "あとで" survives a webview data
    /// wipe, travels with a backup/restore, and does not resurrect the wizard
    /// on another machine once the user has already set the app up.
    ///
    /// This is *not* a mirror of "a provider exists": deleting the only
    /// provider would otherwise re-arm the wizard for someone who already
    /// finished it.
    pub onboarding_completed: Option<bool>,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<SettingsView, String> {
    read_settings(state.storage.pool()).await
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    input: SettingsView,
) -> Result<SettingsView, String> {
    write_settings(state.storage.pool(), input).await
}

/// Read the single settings row, falling back to the defaults.
///
/// Kept separate from the command so the `ON CONFLICT` semantics below can be
/// exercised against a real database.
pub async fn read_settings(pool: &sqlx::Pool<sqlx::Sqlite>) -> Result<SettingsView, String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT data FROM app_settings WHERE id = 'singleton'")
            .fetch_optional(pool)
            .await
            .map_err(crate::error::cmd_err)?;
    Ok(match row {
        Some((data,)) => serde_json::from_str(&data).unwrap_or_default(),
        None => SettingsView::default(),
    })
}

/// Replace the single settings row wholesale.
///
/// `ON CONFLICT DO UPDATE SET data = excluded.data` overwrites the whole
/// document: a field the caller sends as `null` (or an empty allowlist) really
/// does clear what was stored, rather than being merged with the previous row.
pub async fn write_settings(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    input: SettingsView,
) -> Result<SettingsView, String> {
    let json = serde_json::to_string(&input).map_err(crate::error::cmd_err)?;
    sqlx::query(
        "INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, ?) \
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
    )
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(&json)
    .execute(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(input)
}
