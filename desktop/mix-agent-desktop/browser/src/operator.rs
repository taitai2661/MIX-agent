//! Browser Operator: routes [`BrowserCall`]s to the active backend while
//! keeping Workspace-scoped profile state (cookies, localStorage, logins).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::chromiumoxide_backend::ChromiumBackend;
use crate::interface::{BrowserCall, BrowserResponse, BrowserTool};
use crate::profile::{
    default_profile_root, find_bundled_chromium, find_system_chromium, BrowserProfile,
};

/// Backend selection. Kept as an enum so a Firefox / WebKit backend can be
/// added later without changing the Agent-facing surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserBackend {
    Chromium,
    SystemChromium,
    Disabled,
}

pub struct BrowserOperator {
    app_data: PathBuf,
    /// Tauri's resolved resource directory, where the bundled Chromium lives.
    resource_dir: Option<PathBuf>,
    headless: bool,
    backends: RwLock<HashMap<uuid::Uuid, Arc<ChromiumBackend>>>,
    profiles: RwLock<HashMap<uuid::Uuid, BrowserProfile>>,
    artifacts: Mutex<Vec<crate::artifact::BrowserArtifact>>,
}

impl BrowserOperator {
    pub fn new(app_data: impl AsRef<std::path::Path>, headless: bool) -> Self {
        Self::with_resources(app_data, None, headless)
    }

    /// Build an operator that knows where Tauri unpacked its bundled resources.
    pub fn with_resources(
        app_data: impl AsRef<std::path::Path>,
        resource_dir: Option<PathBuf>,
        headless: bool,
    ) -> Self {
        Self {
            app_data: app_data.as_ref().to_path_buf(),
            resource_dir,
            headless,
            backends: RwLock::new(HashMap::new()),
            profiles: RwLock::new(HashMap::new()),
            artifacts: Mutex::new(Vec::new()),
        }
    }

    /// Resolve the Chromium executable: bundled first, then the system copy.
    pub fn resolve_executable(&self) -> (BrowserBackend, Option<PathBuf>) {
        if let Some(exe) = find_bundled_chromium(self.resource_dir.as_deref()) {
            return (BrowserBackend::Chromium, Some(exe));
        }
        if let Some(exe) = find_system_chromium() {
            return (BrowserBackend::SystemChromium, Some(exe));
        }
        (BrowserBackend::Disabled, None)
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
            self.headless,
        ));
        self.profiles.write().await.insert(workspace_id, profile);
        self.backends
            .write()
            .await
            .insert(workspace_id, backend.clone());
        Ok(backend)
    }

    pub async fn profile(&self, workspace_id: uuid::Uuid) -> Option<BrowserProfile> {
        self.profiles.read().await.get(&workspace_id).cloned()
    }

    pub async fn list_profiles(&self) -> Vec<BrowserProfile> {
        self.profiles.read().await.values().cloned().collect()
    }

    /// Wipe a Workspace's cookies and storage.
    pub async fn reset_profile(&self, workspace_id: uuid::Uuid) -> anyhow::Result<()> {
        if let Some(profile) = self.profiles.read().await.get(&workspace_id).cloned() {
            profile.reset().await?;
        }
        self.backends.write().await.remove(&workspace_id);
        self.profiles.write().await.remove(&workspace_id);
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
        let backend = self.ensure_workspace(call.workspace_id, "default").await?;
        // `navigate` is handled by the backend itself; everything else needs
        // the currently open page.
        let response = match call.action.clone() {
            crate::interface::BrowserAction::Navigate { .. }
            | crate::interface::BrowserAction::Info
            | crate::interface::BrowserAction::Close => backend.invoke(call.clone()).await?,
            other => {
                crate::chromiumoxide_backend::perform_on_current_page(
                    &backend,
                    &self.profile(call.workspace_id).await.unwrap_or_else(|| {
                        BrowserProfile::new(
                            call.workspace_id,
                            "default",
                            &default_profile_root(&self.app_data),
                        )
                    }),
                    other,
                )
                .await?
            }
        };
        if let Some(artifact_id) = &response.artifact_id {
            self.artifacts
                .lock()
                .await
                .push(crate::artifact::BrowserArtifact {
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
        }
        Ok(response)
    }

    pub async fn artifacts(&self) -> Vec<crate::artifact::BrowserArtifact> {
        self.artifacts.lock().await.clone()
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
            let _ = backend.invoke(call).await;
        }
        self.backends.write().await.clear();
    }
}
