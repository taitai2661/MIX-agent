//! In-app update check, download and quit-time install.
//!
//! The WebView deliberately never talks to `tauri-plugin-updater` directly.
//! Like every other operation, updates cross the `#[tauri::command]` boundary
//! so the frontend cannot point the updater at an arbitrary payload and so the
//! plugin's own IPC surface stays out of `capabilities/default.toml` (see the
//! least-privilege note there).
//!
//! The flow is deliberately split in two:
//!
//! 1. [`check_for_update`] asks the manifest whether a newer build exists.
//! 2. [`download_update`] downloads and verifies the payload, then stages it
//!    under the app-data directory. It does **not** install anything.
//! 3. When the user quits normally, [`apply_pending_on_exit`] installs the
//!    staged payload. The next launch runs the new build.
//!
//! Windows cannot replace a running executable; the plugin's `install` starts
//! the installer and exits the process. Deferring the install until quit is
//! what lets "download automatically, restart manually" hold on every
//! platform. The process never calls `app.restart()` from the update flow.
//!
//! Signing is mandatory: the plugin verifies every downloaded bundle against
//! the public key in `tauri.conf.json`, and refuses to install anything that
//! does not match. `TAURI_SIGNING_PRIVATE_KEY` / `…_PASSWORD` live only in
//! GitHub Actions Repository Secrets.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Progress event consumed by the Updates settings pane and the header badge.
pub const PROGRESS_EVENT: &str = "update://progress";
/// Emitted once a payload has been downloaded, verified and staged.
pub const READY_EVENT: &str = "update://ready";

/// A manifest fetch must not leave the "更新を確認" button spinning forever
/// when GitHub is unreachable or a proxy silently drops the connection.
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
/// A restored payload has no in-memory `Update`, so applying it on quit needs
/// one manifest check. Keep that check short: quitting must not appear to hang
/// when the machine is offline.
const APPLY_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// The updater's HTTP client has no timeout by default. A stalled download
/// would otherwise pin the pane at "ダウンロード中… 1%" until the app is
/// force-quit.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

const PENDING_BIN: &str = "pending.bin";
const PENDING_JSON: &str = "pending.json";

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
    /// the UI offers the Release download instead of a Download button.
    pub installable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateProgress {
    pub downloaded: u64,
    /// `None` when the server does not send a `Content-Length`.
    pub total: Option<u64>,
    /// `"downloading"` while bytes arrive, then `"verifying"` while the
    /// signature is checked and the payload is written to the cache.
    pub stage: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingMetadata {
    version: String,
    notes: Option<String>,
    date: Option<String>,
    downloaded_at: String,
}

#[derive(Clone)]
struct PendingUpdate {
    info: UpdateInfo,
    /// `Some` for a payload downloaded in this process. A cache restored from
    /// a previous launch has no `Update` yet; applying it re-checks the
    /// manifest so the plugin can rebuild its install context.
    update: Option<Update>,
    path: PathBuf,
}

#[derive(Default)]
struct Inner {
    pending: Option<PendingUpdate>,
    downloading: bool,
    applying: bool,
    /// Set once quit-time install has been attempted, so the second
    /// `ExitRequested` (the one produced by the final `app.exit(0)`) is not
    /// intercepted again and the app cannot loop forever.
    allow_exit: bool,
}

/// Download state shared by the update commands and the quit hook.
pub struct UpdateState {
    dir: PathBuf,
    current_version: String,
    inner: Mutex<Inner>,
}

impl UpdateState {
    /// Create the state and restore a payload staged by a previous process.
    ///
    /// A cache whose version is not newer than the running bundle is stale
    /// (Windows applies an update and exits before the old process can clean
    /// up), so it is removed here rather than offered to the next launch.
    pub fn new(dir: PathBuf, current_version: String) -> Self {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::warn!(%err, dir = %dir.display(), "could not create the update cache directory");
        }
        remove_partials(&dir);
        let pending = load_pending(&dir, &current_version);
        Self {
            dir,
            current_version,
            inner: Mutex::new(Inner {
                pending,
                ..Inner::default()
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// The staged update, if its payload is still present.
    pub fn pending_info(&self) -> Option<UpdateInfo> {
        if !installable_on_this_build() {
            return None;
        }
        let mut inner = self.lock();
        if inner
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.path.is_file())
        {
            inner.pending = None;
            return None;
        }
        inner.pending.as_ref().map(|pending| UpdateInfo {
            current_version: self.current_version.clone(),
            ..pending.info.clone()
        })
    }

    /// Claim the quit-time install if there is a staged payload and no
    /// download/install is already in flight.
    pub fn should_apply_on_exit(&self) -> bool {
        if !installable_on_this_build() {
            return false;
        }
        let mut inner = self.lock();
        if inner.allow_exit || inner.downloading || inner.applying {
            return false;
        }
        if inner
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.path.is_file())
        {
            inner.pending = None;
            return false;
        }
        if inner.pending.is_none() {
            return false;
        }
        inner.applying = true;
        true
    }

    fn finish_exit(&self) {
        let mut inner = self.lock();
        inner.applying = false;
        inner.allow_exit = true;
    }
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
    let updater = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(crate::error::cmd_err)?;
    let update = updater.check().await.map_err(crate::error::cmd_err)?;
    Ok(update.map(|update| info_from_update(&update)))
}

/// Download, verify and stage an update without installing it.
///
/// The install happens on the next normal quit (see [`apply_pending_on_exit`]).
/// A second call while a download is in flight is rejected rather than allowed
/// to race the first one.
#[tauri::command]
pub async fn download_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<UpdateInfo, String> {
    if !installable_on_this_build() {
        return Err(
            "このビルドはアプリ内更新に対応していません。GitHub Releasesから手動で更新してください。"
                .to_string(),
        );
    }
    {
        let mut inner = state.lock();
        if inner.downloading {
            return Err("更新をダウンロード中です。".to_string());
        }
        if inner.applying {
            return Err("更新を適用中です。".to_string());
        }
        inner.downloading = true;
    }
    let result = download_update_inner(&app, state.inner()).await;
    state.lock().downloading = false;
    result
}

async fn download_update_inner(app: &AppHandle, state: &UpdateState) -> Result<UpdateInfo, String> {
    let updater = app
        .updater_builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(crate::error::cmd_err)?;
    let update = updater
        .check()
        .await
        .map_err(crate::error::cmd_err)?
        .ok_or_else(|| "最新バージョンです（更新はありません）".to_string())?;
    let info = info_from_update(&update);

    // A payload restored from a previous launch is already verified. If the
    // manifest still names that version, attach the fresh `Update` and reuse
    // it instead of downloading the same release again. A newer manifest
    // replaces the older staged payload below.
    let mut update = Some(update);
    let reused = {
        let mut inner = state.lock();
        match inner.pending.as_mut() {
            Some(pending) if pending.info.version == info.version && pending.path.is_file() => {
                pending.update = update.take();
                Some(pending.info.clone())
            }
            _ => None,
        }
    };
    if let Some(info) = reused {
        let _ = app.emit(READY_EVENT, &info);
        return Ok(info);
    }
    let update = update.ok_or_else(|| "更新の取得に失敗しました".to_string())?;

    let dir = state.dir.clone();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(crate::error::cmd_err)?;

    let progress_app = app.clone();
    let finish_app = app.clone();
    let mut downloaded = 0u64;
    let bytes = update
        .download(
            move |chunk_len, content_length| {
                downloaded += chunk_len as u64;
                let _ = progress_app.emit(
                    PROGRESS_EVENT,
                    UpdateProgress {
                        downloaded,
                        total: content_length,
                        stage: "downloading",
                    },
                );
            },
            move || {
                // `download` checks the signature after the last chunk; this
                // is the honest label for that window.
                let _ = finish_app.emit(
                    PROGRESS_EVENT,
                    UpdateProgress {
                        downloaded: 0,
                        total: None,
                        stage: "verifying",
                    },
                );
            },
        )
        .await
        .map_err(crate::error::cmd_err)?;

    let bin_path = dir.join(PENDING_BIN);
    let bin_tmp = dir.join("pending.bin.part");
    tokio::fs::write(&bin_tmp, &bytes)
        .await
        .map_err(crate::error::cmd_err)?;
    tokio::fs::rename(&bin_tmp, &bin_path)
        .await
        .map_err(crate::error::cmd_err)?;

    let metadata = PendingMetadata {
        version: info.version.clone(),
        notes: info.notes.clone(),
        date: info.date.clone(),
        downloaded_at: chrono::Utc::now().to_rfc3339(),
    };
    let metadata = serde_json::to_vec_pretty(&metadata).map_err(crate::error::cmd_err)?;
    let json_path = dir.join(PENDING_JSON);
    let json_tmp = dir.join("pending.json.part");
    tokio::fs::write(&json_tmp, metadata)
        .await
        .map_err(crate::error::cmd_err)?;
    tokio::fs::rename(&json_tmp, &json_path)
        .await
        .map_err(crate::error::cmd_err)?;

    {
        let mut inner = state.lock();
        inner.pending = Some(PendingUpdate {
            info: info.clone(),
            update: Some(update),
            path: bin_path,
        });
    }
    let _ = app.emit(READY_EVENT, &info);
    Ok(info)
}

/// The staged update, if one was downloaded earlier in this process or in a
/// previous one. The UI uses it to restore the "next quit applies it" banner.
#[tauri::command]
pub fn pending_update(state: State<'_, UpdateState>) -> Option<UpdateInfo> {
    state.pending_info()
}

/// Apply a staged update and relaunch now.
///
/// The default flow waits for a normal quit so a long Run is not interrupted
/// by an update the user only meant to download. This command is the explicit
/// "restart now" escape hatch, and it refuses while a Run is live.
#[tauri::command]
pub async fn install_update_now(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<(), String> {
    let pending = state
        .lock()
        .pending
        .clone()
        .ok_or_else(|| "ダウンロード済みの更新がありません".to_string())?;
    // Refuse rather than silently killing a coding task. The user can stop
    // the Run first or wait for the normal quit-time install.
    let active: i64 = {
        let storage = app
            .try_state::<crate::AppState>()
            .ok_or_else(|| "app state is not ready".to_string())?;
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM runs WHERE status IN \
             ('queued','running','waiting_approval','budget_extension_pending')",
        )
        .fetch_one(storage.storage.pool())
        .await
        .unwrap_or(0)
    };
    if active > 0 {
        return Err(format!(
            "実行中のタスクが {active} 件あるため再起動できません。先に停止してください。"
        ));
    }
    apply_pending(&app, pending).await?;
    state.lock().pending = None;
    remove_pending_files(&state.dir).await;
    // `install` replaces the bundle on macOS/Linux; on Windows it starts the
    // installer. Restart so the new build is what the user sees next.
    app.restart()
}

/// Install a staged payload while the app is exiting.
///
/// Called only from the `ExitRequested` hook after
/// [`UpdateState::should_apply_on_exit`] claimed the attempt. No automatic
/// relaunch happens here: macOS/Linux replace the bundle and exit, Windows
/// starts the installer with `restart_after_install(false)` and exits. The
/// next launch is the user's own.
pub async fn apply_pending_on_exit(app: AppHandle) {
    let Some(state) = app.try_state::<UpdateState>() else {
        return;
    };
    let pending = state.lock().pending.clone();
    if let Some(pending) = pending {
        match apply_pending(&app, pending).await {
            Ok(()) => {
                state.lock().pending = None;
                remove_pending_files(&state.dir).await;
            }
            Err(err) => {
                // Keep the payload: the next normal quit retries. A failed
                // install must never keep the app from exiting.
                tracing::warn!(%err, "could not apply the downloaded update on exit");
            }
        }
    }
    state.finish_exit();
}

async fn apply_pending(app: &AppHandle, pending: PendingUpdate) -> Result<(), String> {
    let bytes = tokio::fs::read(&pending.path)
        .await
        .map_err(crate::error::cmd_err)?;
    let update = match pending.update {
        Some(update) => update,
        None => {
            // Restored from disk: the plugin needs a fresh `Update` (its
            // install context is not serializable). Offline exits keep the
            // payload and try again next time.
            let updater = app
                .updater_builder()
                .timeout(APPLY_CHECK_TIMEOUT)
                .build()
                .map_err(crate::error::cmd_err)?;
            let Some(update) = updater.check().await.map_err(crate::error::cmd_err)? else {
                return Err("更新はすでに最新です".to_string());
            };
            if update.version != pending.info.version {
                return Err(format!(
                    "the staged version v{} is no longer offered (latest is v{})",
                    pending.info.version, update.version
                ));
            }
            update
        }
    };

    update.verify(&bytes).map_err(crate::error::cmd_err)?;
    // `restart_after_install(false)` is a no-op outside Windows, where it
    // stops the installer from relaunching the app. The user asked for a
    // manual next launch.
    update
        .clone()
        .restart_after_install(false)
        .install(&bytes)
        .map_err(crate::error::cmd_err)?;
    Ok(())
}

fn info_from_update(update: &Update) -> UpdateInfo {
    UpdateInfo {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        notes: update.body.clone(),
        // `OffsetDateTime`'s Display is RFC 3339-shaped and stable enough for
        // a human-readable "公開日"; the UI never parses it.
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

fn load_pending(dir: &Path, current_version: &str) -> Option<PendingUpdate> {
    // A `.deb` build cannot replace itself, and a cache written by an AppImage
    // install on the same machine must not be offered after switching packages.
    if !installable_on_this_build() {
        remove_pending_files_sync(dir);
        return None;
    }
    let json_path = dir.join(PENDING_JSON);
    let bin_path = dir.join(PENDING_BIN);
    let raw = match std::fs::read_to_string(&json_path) {
        Ok(raw) => raw,
        Err(_) => {
            // An orphaned payload cannot be described to the UI or verified
            // against a manifest version.
            let _ = std::fs::remove_file(&bin_path);
            return None;
        }
    };
    let metadata: PendingMetadata = match serde_json::from_str(&raw) {
        Ok(metadata) => metadata,
        Err(err) => {
            tracing::warn!(%err, "discarding an unreadable staged update");
            remove_pending_files_sync(dir);
            return None;
        }
    };
    if !bin_path.is_file() {
        remove_pending_files_sync(dir);
        return None;
    }
    if !is_newer_version(&metadata.version, current_version) {
        remove_pending_files_sync(dir);
        return None;
    }
    Some(PendingUpdate {
        info: UpdateInfo {
            current_version: current_version.to_string(),
            version: metadata.version,
            notes: metadata.notes,
            date: metadata.date,
            installable: installable_on_this_build(),
        },
        update: None,
        path: bin_path,
    })
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Ok(candidate), Ok(current)) => candidate > current,
        // A non-semver version is kept unless it is exactly what is running;
        // discarding a valid payload over a parse failure would be worse.
        _ => candidate != current,
    }
}

fn remove_partials(dir: &Path) {
    for name in ["pending.bin.part", "pending.json.part"] {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

fn remove_pending_files_sync(dir: &Path) {
    for name in [PENDING_BIN, PENDING_JSON] {
        let _ = std::fs::remove_file(dir.join(name));
    }
    remove_partials(dir);
}

async fn remove_pending_files(dir: &Path) {
    for name in [PENDING_BIN, PENDING_JSON] {
        let _ = tokio::fs::remove_file(dir.join(name)).await;
    }
    for name in ["pending.bin.part", "pending.json.part"] {
        let _ = tokio::fs::remove_file(dir.join(name)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::is_newer_version;
    #[cfg(not(target_os = "linux"))]
    use super::{PendingMetadata, UpdateState, PENDING_BIN, PENDING_JSON};
    #[cfg(not(target_os = "linux"))]
    use std::path::Path;

    #[test]
    fn only_a_newer_staged_version_is_restored() {
        assert!(is_newer_version("0.5.7", "0.5.6"));
        assert!(is_newer_version("0.6.0", "0.5.6"));
        assert!(!is_newer_version("0.5.6", "0.5.6"));
        assert!(!is_newer_version("0.5.5", "0.5.6"));
        // Unknown formats are kept unless they name the running version.
        assert!(is_newer_version("nightly", "0.5.6"));
        assert!(!is_newer_version("nightly", "nightly"));
    }

    #[cfg(not(target_os = "linux"))]
    fn temp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("mixd-update-test-{}", uuid::Uuid::new_v4()))
    }

    #[cfg(not(target_os = "linux"))]
    fn write_pending(dir: &Path, version: &str) {
        std::fs::create_dir_all(dir).expect("create cache dir");
        std::fs::write(dir.join(PENDING_BIN), b"verified payload").expect("write payload");
        let metadata = PendingMetadata {
            version: version.to_string(),
            notes: Some("notes".to_string()),
            date: None,
            downloaded_at: "2026-01-01T00:00:00Z".to_string(),
        };
        std::fs::write(
            dir.join(PENDING_JSON),
            serde_json::to_vec(&metadata).expect("serialize metadata"),
        )
        .expect("write metadata");
    }

    // Linux without `APPIMAGE` is a `.deb` install, where the updater is
    // intentionally disabled. The pure version test above still covers that
    // comparison there.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn restores_a_newer_payload_and_discards_a_stale_one() {
        let dir = temp_dir();
        write_pending(&dir, "9.9.9");

        let state = UpdateState::new(dir.clone(), "0.5.6".to_string());
        assert_eq!(
            state.pending_info().map(|info| info.version),
            Some("9.9.9".to_string())
        );
        assert!(state.should_apply_on_exit());

        // A running build at or above the staged version means the payload is
        // stale (Windows exits before it can clean the cache).
        let state = UpdateState::new(dir.clone(), "9.9.9".to_string());
        assert!(state.pending_info().is_none());
        assert!(!state.should_apply_on_exit());

        let _ = std::fs::remove_dir_all(dir);
    }
}
