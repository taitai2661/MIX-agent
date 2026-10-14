//! Auto-update bootstrap.
//!
//! Checks for a new release on launch and, when one exists, asks the frontend
//! to show the update banner. Installation is driven from the UI so the user
//! stays in control and a failed download never restarts the app unexpectedly.
//!
//! Both the updater endpoint and the signing public key are optional: a local
//! build has neither, and `install` must stay a no-op rather than fail startup.

use tauri::{AppHandle, Emitter, Runtime};

/// How long to wait after launch before checking, so a slow start is not
/// competing with an update download.
const CHECK_DELAY_SECS: u64 = 20;

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateAvailable {
    pub version: String,
    pub current_version: String,
    pub notes: String,
    pub date: Option<String>,
}

/// Is an updater endpoint configured?
///
/// `PluginConfig` is a name-keyed map, so the updater block is looked up by key
/// rather than through a typed field.
pub fn is_configured<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|v| v.get("endpoints"))
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
}

/// Check once and emit `update_available` if there is a newer release.
pub async fn check<R: Runtime>(app: &AppHandle<R>) -> Result<Option<UpdateAvailable>, String> {
    use tauri_plugin_updater::UpdaterExt;

    if !is_configured(app) {
        return Ok(None);
    }
    let updater = app.updater().map_err(crate::error::cmd_err)?;
    let _ = &updater;
    let current = env!("CARGO_PKG_VERSION").to_string();
    match updater.check().await {
        Ok(Some(update)) => {
            let info = UpdateAvailable {
                version: update.version.clone(),
                current_version: current,
                notes: update.body.clone().unwrap_or_default(),
                date: update.date.map(|d| d.to_string()),
            };
            let _ = app.emit("update_available", &info);
            Ok(Some(info))
        }
        Ok(None) => Ok(None),
        Err(err) => Err(crate::error::cmd_err(err)),
    }
}

/// Download and install the pending update, then relaunch.
pub async fn install_and_relaunch<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    if !is_configured(app) {
        return Err("no update endpoint is configured".into());
    }
    let updater = app.updater().map_err(crate::error::cmd_err)?;
    let update = updater
        .check()
        .await
        .map_err(crate::error::cmd_err)?
        .ok_or_else(|| "no update is available".to_string())?;
    update
        .download_and_install(
            |chunk, total| {
                let _ = app.emit("update_progress", (chunk, total));
            },
            || {},
        )
        .await
        .map_err(crate::error::cmd_err)?;
    app.restart();
}

pub fn install<R: Runtime>(app: &AppHandle<R>) {
    if !is_configured(app) {
        tracing::info!("auto-update is not configured; skipping the launch check");
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(CHECK_DELAY_SECS)).await;
        if let Err(err) = check(&handle).await {
            tracing::info!(?err, "update check failed");
        }
    });
}

/// Check on demand (the Help ▸ Check for Updates menu item).
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Option<UpdateAvailable>, String> {
    check(&app).await
}

/// Download, install and relaunch.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    install_and_relaunch(&app).await
}
