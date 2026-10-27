//! In-app update check and install.
//!
//! The WebView deliberately never talks to `tauri-plugin-updater` directly.
//! Like every other operation, updates cross the `#[tauri::command]` boundary
//! so the frontend cannot point the updater at an arbitrary payload and so the
//! plugin's own IPC surface stays out of `capabilities/default.toml` (see the
//! least-privilege note there).
//!
//! Signing is mandatory: the plugin verifies every downloaded bundle against
//! the public key in `tauri.conf.json`, and refuses to install anything that
//! does not match. `TAURI_SIGNING_PRIVATE_KEY` / `…_PASSWORD` live only in
//! GitHub Actions Repository Secrets.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// Progress event consumed by the Updates settings pane.
pub const PROGRESS_EVENT: &str = "update://progress";
/// Emitted once the payload has been verified and installed.
pub const INSTALLED_EVENT: &str = "update://installed";

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    /// Version currently running (from the bundle metadata).
    pub current_version: String,
    /// Version announced by `latest.json`.
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
    /// False on a Linux build that is not running as an AppImage (typically a
    /// `.deb` install). The plugin cannot replace a distro-managed package, so
    /// the UI offers the Release download instead of an Install button.
    pub installable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateProgress {
    pub downloaded: u64,
    /// `None` when the server does not send a `Content-Length`.
    pub total: Option<u64>,
}

/// The running app version, so the pane can show it without a second plugin.
#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Ask the configured endpoint whether a newer version exists.
///
/// `Ok(None)` means "already current" and is not an error: the UI renders it
/// as such instead of a red banner.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(crate::error::cmd_err)?;
    let update = updater.check().await.map_err(crate::error::cmd_err)?;
    Ok(update.map(|update| info_from_update(&update)))
}

/// Re-check, then download, verify and install the update.
///
/// Re-checking instead of caching the `Update` in managed state keeps the
/// command self-contained: a stale manifest between "check" and "install" is
/// re-read, and the signature/version the plugin verifies is the one it just
/// fetched. On Windows the installer exits the process; macOS and Linux need
/// the caller to relaunch afterwards (`relaunch_app`).
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<UpdateInfo, String> {
    if !installable_on_this_build() {
        return Err(
            "このビルドはアプリ内更新に対応していません。GitHub Releasesから手動で更新してください。"
                .to_string(),
        );
    }
    let updater = app.updater().map_err(crate::error::cmd_err)?;
    let update = updater
        .check()
        .await
        .map_err(crate::error::cmd_err)?
        .ok_or_else(|| "最新バージョンです（更新はありません）".to_string())?;
    let info = info_from_update(&update);

    let progress_handle = app.clone();
    let mut downloaded = 0u64;
    update
        .download_and_install(
            move |chunk_len, content_length| {
                downloaded += chunk_len as u64;
                let _ = progress_handle.emit(
                    PROGRESS_EVENT,
                    UpdateProgress {
                        downloaded,
                        total: content_length,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(crate::error::cmd_err)?;

    let _ = app.emit(INSTALLED_EVENT, &info);
    Ok(info)
}

fn info_from_update(update: &tauri_plugin_updater::Update) -> UpdateInfo {
    UpdateInfo {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        notes: update.body.clone(),
        // `OffsetDateTime`'s Display is RFC 3339-shaped and stable enough for a
        // human-readable "公開日"; the UI never parses it.
        date: update.date.map(|date| date.to_string()),
        installable: installable_on_this_build(),
    }
}

/// Whether this build can apply an update itself.
///
/// Tauri's Linux updater replaces the running AppImage; a `.deb` install is
/// owned by the package manager and must be upgraded manually. `APPIMAGE` is
/// set by the AppImage runtime, so its absence means "not an AppImage".
fn installable_on_this_build() -> bool {
    !cfg!(target_os = "linux") || std::env::var_os("APPIMAGE").is_some()
}
