//! Browser Operator: routes [`BrowserCall`]s to the active backend while
//! keeping Workspace-scoped profile state (cookies, localStorage, logins).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, RwLock};

use serde::Serialize;

use crate::chromiumoxide_backend::ChromiumBackend;
use crate::interface::{BrowserCall, BrowserFrame, BrowserResponse, BrowserTool, FrameSink};
use crate::profile::{
    default_profile_root, find_bundled_chromium, find_downloaded_chromium, find_override_chromium,
    find_system_chromium, BrowserProfile,
};

/// Backend selection. Kept as an enum so a Firefox / WebKit backend can be
/// added later without changing the Agent-facing surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserBackend {
    Chromium,
    SystemChromium,
    Disabled,
}

/// Where the resolved executable came from. The UI uses this to decide
/// whether it may offer update/remove actions (managed installs only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSource {
    Installed,
    Bundled,
    System,
    Override,
    None,
}

/// Cap on the in-memory artifact index. Screenshots are the main producer and
/// a long browser session would otherwise grow it without bound.
const MAX_ARTIFACTS: usize = 500;
/// A Chromium instance is hundreds of megabytes. Keep only the workspaces the
/// user is plausibly working in; the profile on disk survives eviction, so
/// reopening restores cookies and logins.
pub const DEFAULT_MAX_OPEN_WORKSPACES: usize = 2;
/// Close a browser that has not been touched by the agent or the live view.
/// An open pane sends a periodic keepalive, so it only expires after the pane
/// is closed.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Memory/CPU-constrained machines can lower the live-browser cap without a
/// rebuild; values below 1 are ignored.
pub fn max_open_workspaces() -> usize {
    std::env::var("MIX_BROWSER_MAX_OPEN")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 1)
        .unwrap_or(DEFAULT_MAX_OPEN_WORKSPACES)
}

/// Idle timeout override, in seconds (`MIX_BROWSER_IDLE_SECS`).
pub fn idle_timeout() -> Duration {
    std::env::var("MIX_BROWSER_IDLE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value >= 30)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_IDLE_TIMEOUT)
}

pub struct BrowserOperator {
    app_data: PathBuf,
    /// Tauri's resolved resource directory, for the legacy bundled Chromium.
    resource_dir: Option<PathBuf>,
    backends: RwLock<HashMap<uuid::Uuid, Arc<ChromiumBackend>>>,
    /// Last agent/live-view call per workspace, used for LRU + idle eviction.
    last_used: Mutex<HashMap<uuid::Uuid, Instant>>,
    /// App-layer sink for CDP screencast frames (a Tauri event in production).
    frame_sink: RwLock<Option<FrameSink>>,
    profiles: RwLock<HashMap<uuid::Uuid, BrowserProfile>>,
    /// Serialises workspace backend creation so two simultaneous calls cannot
    /// each build one and leak the loser's Chromium instance.
    creation: Mutex<()>,
    /// Serialises every action for one workspace. Two concurrent actions on
    /// the same tab would otherwise interleave navigation and input.
    workspace_locks: Mutex<HashMap<uuid::Uuid, Arc<Mutex<()>>>>,
    artifacts: Mutex<Vec<crate::artifact::BrowserArtifact>>,
    /// Persisted artifact index. The files stay on disk either way; without
    /// this the UI loses every historical screenshot after a restart.
    artifacts_path: PathBuf,
    /// Explicit opt-in for loopback / RFC1918 navigation.
    allow_local: AtomicBool,
}

impl BrowserOperator {
    pub fn new(app_data: impl AsRef<std::path::Path>) -> Self {
        Self::with_resources(app_data, None)
    }

    /// Build an operator that knows where Tauri unpacked its bundled resources.
    pub fn with_resources(
        app_data: impl AsRef<std::path::Path>,
        resource_dir: Option<PathBuf>,
    ) -> Self {
        let app_data = app_data.as_ref().to_path_buf();
        let artifacts_path = app_data.join("artifacts").join("index.json");
        let mut artifacts = std::fs::read_to_string(&artifacts_path)
            .ok()
            .and_then(|raw| {
                serde_json::from_str::<Vec<crate::artifact::BrowserArtifact>>(&raw).ok()
            })
            .unwrap_or_default();
        // A crash between writing a PNG and persisting the index leaves an
        // entry whose file is gone; drop it now so the UI does not render a
        // permanently broken thumbnail. The reverse (a file with no entry) is
        // intentionally left alone: it may be the screenshot being written
        // right now, and the index is the source of truth the UI reads.
        artifacts.retain(|artifact| std::path::Path::new(&artifact.path).is_file());
        // Older builds capped the in-memory list only; trim a persisted index
        // that exceeds the current cap and remove the evicted files.
        if artifacts.len() > MAX_ARTIFACTS {
            let excess = artifacts.len() - MAX_ARTIFACTS;
            for artifact in artifacts.drain(..excess) {
                let _ = std::fs::remove_file(&artifact.path);
            }
        }
        Self {
            app_data,
            resource_dir,
            backends: RwLock::new(HashMap::new()),
            last_used: Mutex::new(HashMap::new()),
            frame_sink: RwLock::new(None),
            profiles: RwLock::new(HashMap::new()),
            creation: Mutex::new(()),
            workspace_locks: Mutex::new(HashMap::new()),
            artifacts: Mutex::new(artifacts),
            artifacts_path,
            allow_local: AtomicBool::new(false),
        }
    }

    /// Resolve the Chromium executable and where it came from.
    ///
    /// A managed install wins over the legacy bundle: after an app update the
    /// freshly downloaded browser must be the one the agent drives, not a
    /// stale copy that happened to survive in the old bundle. The developer
    /// override is first so a fixture can be tested without touching app-data.
    pub fn resolve(&self) -> (BrowserBackend, Option<PathBuf>, BrowserSource) {
        if let Some(exe) = find_override_chromium() {
            return (BrowserBackend::Chromium, Some(exe), BrowserSource::Override);
        }
        if let Some(exe) = find_downloaded_chromium(&self.app_data) {
            return (
                BrowserBackend::Chromium,
                Some(exe),
                BrowserSource::Installed,
            );
        }
        if let Some(exe) = find_bundled_chromium(self.resource_dir.as_deref()) {
            return (BrowserBackend::Chromium, Some(exe), BrowserSource::Bundled);
        }
        if let Some(exe) = find_system_chromium() {
            return (
                BrowserBackend::SystemChromium,
                Some(exe),
                BrowserSource::System,
            );
        }
        (BrowserBackend::Disabled, None, BrowserSource::None)
    }

    /// Backwards-compatible tuple for call sites that only need backend + path.
    pub fn resolve_executable(&self) -> (BrowserBackend, Option<PathBuf>) {
        let (backend, executable, _) = self.resolve();
        (backend, executable)
    }

    pub async fn set_allow_local(&self, value: bool) {
        self.allow_local.store(value, Ordering::Relaxed);
        // Existing backends have already copied the flag, so update them too.
        for backend in self.backends.read().await.values() {
            backend.set_allow_local(value);
        }
    }

    pub fn allows_local(&self) -> bool {
        self.allow_local.load(Ordering::Relaxed)
    }

    /// Install the app-layer sink that turns CDP screencast frames into UI
    /// events. Called once during startup; existing backends keep their own
    /// clone, so this must happen before the first workspace is created.
    pub async fn set_frame_sink(&self, sink: FrameSink) {
        *self.frame_sink.write().await = Some(sink);
    }

    async fn touch(&self, workspace_id: uuid::Uuid) {
        self.last_used
            .lock()
            .await
            .insert(workspace_id, Instant::now());
    }

    async fn lock_for(&self, workspace_id: uuid::Uuid) -> Arc<Mutex<()>> {
        self.workspace_locks
            .lock()
            .await
            .entry(workspace_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Create (or fetch) the Workspace profile and its backend.
    pub async fn ensure_workspace(
        &self,
        workspace_id: uuid::Uuid,
        display_name: &str,
    ) -> anyhow::Result<Arc<ChromiumBackend>> {
        if let Some(existing) = self.backends.read().await.get(&workspace_id) {
            return Ok(existing.clone());
        }
        // Re-check under the creation lock: without it two concurrent calls can
        // both miss the read above, each build a backend, and one then overwrite
        // the other in the map, orphaning a Chromium instance.
        let _creation = self.creation.lock().await;
        if let Some(existing) = self.backends.read().await.get(&workspace_id) {
            return Ok(existing.clone());
        }
        let root = default_profile_root(&self.app_data);
        let mut profile = BrowserProfile::new(workspace_id, display_name, &root);
        profile.ensure().await?;

        let (_, executable) = self.resolve_executable();
        if executable.is_none() {
            tracing::warn!(
                workspace = %workspace_id,
                "no bundled or system Chromium found; the browser tool will fail to start"
            );
        }
        let artifact_dir = self
            .app_data
            .join("artifacts")
            .join(workspace_id.to_string());
        let backend = Arc::new(ChromiumBackend::new(
            executable,
            Some(profile.path()),
            artifact_dir,
            workspace_id,
            self.frame_sink.read().await.clone(),
        ));
        backend.set_allow_local(self.allows_local());
        self.profiles.write().await.insert(workspace_id, profile);
        let mut backends = self.backends.write().await;
        backends.insert(workspace_id, backend.clone());
        // Bound the number of live Chromium trees. Evict the least recently
        // used workspace other than the one that was just requested; its
        // on-disk profile keeps cookies and logins for the next open.
        let victim = if backends.len() > max_open_workspaces() {
            let used = self.last_used.lock().await;
            backends
                .keys()
                .filter(|id| **id != workspace_id)
                .min_by_key(|id| used.get(*id).copied())
                .map(|id| (*id, used.get(id).copied()))
        } else {
            None
        };
        drop(backends);
        // The victim's workspace lock must be acquired *before* closing, but
        // the creation lock must not be held while waiting for it: an action
        // for the victim could itself be waiting for the creation lock, which
        // would deadlock the two.
        drop(_creation);
        if let Some((victim, seen_at)) = victim {
            self.close_workspace_if_still_stale(victim, seen_at).await;
        }
        self.touch(workspace_id).await;
        Ok(backend)
    }

    /// Close an LRU victim only if it has not been used since it was selected.
    ///
    /// Selecting a victim and acquiring its workspace lock are separate steps;
    /// in between, another action (or live-view poll) may have started using
    /// it. Closing then would fail that call and force a relaunch.
    async fn close_workspace_if_still_stale(
        &self,
        workspace_id: uuid::Uuid,
        seen_at: Option<Instant>,
    ) {
        let lock = self.lock_for(workspace_id).await;
        let _guard = lock.lock().await;
        let current = self.last_used.lock().await.get(&workspace_id).copied();
        if current != seen_at {
            return;
        }
        self.close_workspace_locked(workspace_id).await;
    }

    /// Close one workspace's Chromium without deleting its profile or
    /// artifacts. The next call relaunches it lazily.
    ///
    /// The per-workspace lock is held for the whole close.  Removing the
    /// backend and killing Chromium without it could race an in-flight agent
    /// action (or a live-view capture) and fail an otherwise valid call.
    pub async fn close_workspace(&self, workspace_id: uuid::Uuid) {
        let lock = self.lock_for(workspace_id).await;
        let _guard = lock.lock().await;
        self.close_workspace_locked(workspace_id).await;
    }

    async fn close_workspace_locked(&self, workspace_id: uuid::Uuid) {
        let backend = self.backends.write().await.remove(&workspace_id);
        self.last_used.lock().await.remove(&workspace_id);
        if let Some(backend) = backend {
            let call = BrowserCall {
                workspace_id,
                action: crate::interface::BrowserAction::Close,
            };
            if let Err(err) = backend.invoke(call).await {
                tracing::warn!(%err, workspace = %workspace_id, "closing an idle browser failed");
            }
        }
    }

    /// Mark a workspace as recently used without doing any CDP work.
    ///
    /// The side panel calls this while it is open, including while the live
    /// view is paused.  Polling `info` would launch Chromium just to keep it
    /// alive; a keepalive only updates the LRU/idle timestamp.
    pub async fn touch_workspace(&self, workspace_id: uuid::Uuid) {
        self.touch(workspace_id).await;
    }

    /// Close every workspace whose last call is older than `max_idle`.
    /// Intended to be called from a periodic app-level task.
    pub async fn close_idle(&self, max_idle: Duration) {
        let now = Instant::now();
        let victims: Vec<uuid::Uuid> = {
            let used = self.last_used.lock().await;
            used.iter()
                .filter(|(_, at)| now.duration_since(**at) >= max_idle)
                .map(|(id, _)| *id)
                .collect()
        };
        for id in victims {
            let lock = self.lock_for(id).await;
            let _guard = lock.lock().await;
            // The workspace may have been touched (or closed) between building
            // the victim list and acquiring the lock.
            let still_idle = self
                .last_used
                .lock()
                .await
                .get(&id)
                .is_some_and(|at| now.duration_since(*at) >= max_idle);
            if !still_idle {
                continue;
            }
            self.close_workspace_locked(id).await;
        }
    }

    pub async fn profile(&self, workspace_id: uuid::Uuid) -> Option<BrowserProfile> {
        self.profiles.read().await.get(&workspace_id).cloned()
    }

    /// Profiles known to this session plus directories that survived a restart.
    ///
    /// The in-memory map alone used to forget every existing profile when the
    /// app relaunched, so the pane showed an empty picker even though the
    /// cookies and logins were still on disk.
    pub async fn list_profiles(&self) -> Vec<BrowserProfile> {
        let mut out: Vec<BrowserProfile> = self.profiles.read().await.values().cloned().collect();
        let root = default_profile_root(&self.app_data);
        if let Ok(mut entries) = tokio::fs::read_dir(&root).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let Ok(id) = uuid::Uuid::parse_str(&name) else {
                    continue;
                };
                if out.iter().any(|profile| profile.workspace_id == id) {
                    continue;
                }
                if entry.path().is_dir() {
                    out.push(BrowserProfile {
                        workspace_id: id,
                        display_name: "default".to_string(),
                        directory: entry.path().to_string_lossy().to_string(),
                        provisioned: true,
                    });
                }
            }
        }
        out.sort_by_key(|profile| profile.workspace_id);
        out
    }

    /// Wipe a Workspace's cookies and storage.
    pub async fn reset_profile(&self, workspace_id: uuid::Uuid) -> anyhow::Result<()> {
        let lock = self.lock_for(workspace_id).await;
        let _guard = lock.lock().await;
        // Close Chromium before deleting the user-data directory; otherwise the
        // process keeps the profile open and can recreate files after reset.
        if let Some(backend) = self.backends.write().await.remove(&workspace_id) {
            let _ = backend
                .invoke(BrowserCall {
                    workspace_id,
                    action: crate::interface::BrowserAction::Close,
                })
                .await;
        }
        if let Some(profile) = self.profiles.read().await.get(&workspace_id).cloned() {
            profile.reset().await?;
        } else {
            // A profile restored from disk after a restart is not in the map.
            let profile = BrowserProfile::new(
                workspace_id,
                "default",
                &default_profile_root(&self.app_data),
            );
            profile.reset().await?;
        }
        self.profiles.write().await.remove(&workspace_id);
        self.workspace_locks.lock().await.remove(&workspace_id);
        Ok(())
    }

    /// The single entry point the Tool Registry calls.
    ///
    /// Logs the failure with the workspace and action it belongs to: the
    /// Registry only knows the tool name, which is not enough to tell one
    /// browser call from another when a run goes wrong.
    pub async fn invoke(&self, call: BrowserCall) -> anyhow::Result<BrowserResponse> {
        let response = self.invoke_call(&call).await;
        if let Err(err) = &response {
            tracing::warn!(
                workspace = %call.workspace_id,
                action = ?call.action,
                %err,
                "browser call failed"
            );
        }
        response
    }

    async fn invoke_call(&self, call: &BrowserCall) -> anyhow::Result<BrowserResponse> {
        // Every action for a workspace is serialised. The agent and the manual
        // pane share one Chromium profile, so an unguarded click racing a
        // navigation used to land on whichever page CDP happened to report
        // first.
        let lock = self.lock_for(call.workspace_id).await;
        let _guard = lock.lock().await;
        let backend = self.ensure_workspace(call.workspace_id, "default").await?;
        self.touch(call.workspace_id).await;
        let response = backend.invoke(call.clone()).await?;
        if let Some(artifact_id) = &response.artifact_id {
            let mut artifacts = self.artifacts.lock().await;
            artifacts.push(crate::artifact::BrowserArtifact {
                id: artifact_id.clone(),
                run_id: call.workspace_id,
                kind: crate::artifact::ArtifactKind::Screenshot,
                mime_type: "image/png".into(),
                path: self
                    .app_data
                    .join("artifacts")
                    .join(call.workspace_id.to_string())
                    .join(format!("{}.png", artifact_id))
                    .to_string_lossy()
                    .to_string(),
            });
            // Drop the oldest entries once the index is full and delete their
            // PNGs. Keeping the index bounded while leaving every file behind
            // made a long-running browser session grow the disk without limit.
            if artifacts.len() > MAX_ARTIFACTS {
                let excess = artifacts.len() - MAX_ARTIFACTS;
                for artifact in artifacts.drain(..excess) {
                    if let Err(err) = tokio::fs::remove_file(&artifact.path).await {
                        tracing::warn!(%err, path = %artifact.path, "removing an evicted browser artifact failed");
                    }
                }
            }
            self.persist_artifacts(&artifacts).await;
        }
        Ok(response)
    }

    async fn persist_artifacts(&self, artifacts: &[crate::artifact::BrowserArtifact]) {
        let Some(parent) = self.artifacts_path.parent() else {
            return;
        };
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            tracing::warn!(%err, "creating the browser artifact directory failed");
            return;
        }
        match serde_json::to_vec(artifacts) {
            Ok(bytes) => {
                if let Err(err) = tokio::fs::write(&self.artifacts_path, bytes).await {
                    tracing::warn!(%err, "writing the browser artifact index failed");
                }
            }
            Err(err) => tracing::warn!(%err, "serialising the browser artifact index failed"),
        }
    }

    pub async fn artifacts(&self) -> Vec<crate::artifact::BrowserArtifact> {
        self.artifacts.lock().await.clone()
    }

    /// One JPEG for the live-view poller. This does not create an artifact, so
    /// polling cannot grow the index or the disk.
    ///
    /// The capture shares the workspace lock with agent actions (a frame taken
    /// during a half-finished navigation would be misleading), but uses
    /// `try_lock`: a busy workspace yields `None` and the UI keeps the previous
    /// frame instead of making the agent's click wait behind a screenshot. A
    /// 1.5 s poll that blocked each action would be a self-inflicted latency.
    pub async fn capture_frame(
        &self,
        workspace_id: uuid::Uuid,
        full_page: bool,
    ) -> anyhow::Result<Option<BrowserFrame>> {
        let lock = self.lock_for(workspace_id).await;
        let Ok(_guard) = lock.try_lock() else {
            return Ok(None);
        };
        let backend = self.ensure_workspace(workspace_id, "default").await?;
        self.touch(workspace_id).await;
        backend.capture_frame(full_page).await.map(Some)
    }

    /// Stop loading the active tab from the side-panel toolbar.
    pub async fn stop_loading(&self, workspace_id: uuid::Uuid) -> anyhow::Result<()> {
        let lock = self.lock_for(workspace_id).await;
        let _guard = lock.lock().await;
        let backend = self.ensure_workspace(workspace_id, "default").await?;
        self.touch(workspace_id).await;
        backend.stop_loading().await
    }

    /// Start pushing CDP screencast frames for one workspace.
    pub async fn start_screencast(&self, workspace_id: uuid::Uuid) -> anyhow::Result<()> {
        // CDP commands for one tab must not interleave with an agent action.
        let lock = self.lock_for(workspace_id).await;
        let _guard = lock.lock().await;
        let backend = self.ensure_workspace(workspace_id, "default").await?;
        self.touch(workspace_id).await;
        backend.start_screencast().await
    }

    pub async fn stop_screencast(&self, workspace_id: uuid::Uuid) {
        // A stop is still a CDP command; serialise it with agent actions so it
        // cannot interleave with a navigation or click.
        let lock = self.lock_for(workspace_id).await;
        let _guard = lock.lock().await;
        if let Some(backend) = self.backends.read().await.get(&workspace_id).cloned() {
            backend.stop_screencast().await;
        }
    }

    /// Close every open browser and drop the cached backends so the next call
    /// re-resolves the executable. Used after installing/removing Chromium.
    pub async fn reset_backends(&self) {
        self.shutdown_all().await;
    }

    /// Close every open browser (called on app shutdown).
    pub async fn shutdown_all(&self) {
        let backends: Vec<Arc<ChromiumBackend>> =
            self.backends.read().await.values().cloned().collect();
        for backend in backends {
            let call = BrowserCall {
                workspace_id: uuid::Uuid::nil(),
                action: crate::interface::BrowserAction::Close,
            };
            // A failed close leaks a Chromium process, and clearing the map
            // right after drops the only handle that could retry it, so the
            // failure is logged rather than discarded.
            if let Err(err) = backend.invoke(call).await {
                tracing::warn!(%err, "closing a Chromium backend during shutdown failed");
            }
        }
        self.backends.write().await.clear();
        self.last_used.lock().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{ArtifactKind, BrowserArtifact};

    fn artifact_at(root: &std::path::Path, index: usize) -> BrowserArtifact {
        let dir = root.join("artifacts").join(uuid::Uuid::nil().to_string());
        std::fs::create_dir_all(&dir).expect("artifact dir");
        let path = dir.join(format!("{index}.png"));
        std::fs::write(&path, b"png").expect("artifact file");
        BrowserArtifact {
            id: index.to_string(),
            run_id: uuid::Uuid::nil(),
            kind: ArtifactKind::Screenshot,
            mime_type: "image/png".into(),
            path: path.to_string_lossy().to_string(),
        }
    }

    fn write_index(root: &std::path::Path, index: &[BrowserArtifact]) {
        std::fs::write(
            root.join("artifacts/index.json"),
            serde_json::to_vec(index).expect("serialize"),
        )
        .expect("index");
    }

    #[tokio::test]
    async fn startup_drops_index_entries_whose_file_is_gone() {
        let root = std::env::temp_dir().join(format!("mixd-artifacts-{}", uuid::Uuid::new_v4()));
        let kept = artifact_at(&root, 1);
        let mut gone = artifact_at(&root, 2);
        std::fs::remove_file(&gone.path).expect("remove file");
        gone.id = "gone".into();
        write_index(&root, &[kept.clone(), gone]);

        let operator = BrowserOperator::new(&root);
        let artifacts = operator.artifacts().await;
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].id, kept.id);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn startup_prunes_the_index_and_deletes_evicted_files() {
        let root = std::env::temp_dir().join(format!("mixd-artifacts-{}", uuid::Uuid::new_v4()));
        let index: Vec<BrowserArtifact> =
            (0..=MAX_ARTIFACTS).map(|i| artifact_at(&root, i)).collect();
        write_index(&root, &index);

        let operator = BrowserOperator::new(&root);
        let artifacts = operator.artifacts().await;
        assert_eq!(artifacts.len(), MAX_ARTIFACTS);
        assert!(
            !std::path::Path::new(&index[0].path).exists(),
            "the oldest evicted screenshot must be deleted"
        );
        assert!(
            std::path::Path::new(&index[MAX_ARTIFACTS].path).exists(),
            "the newest screenshot must survive"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
