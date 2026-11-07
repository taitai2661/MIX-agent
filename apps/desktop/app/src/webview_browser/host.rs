//! Tauri child-WebView implementation of [`mixd_browser::BrowserHost`].
//!
//! One native child WebView per tab. The React shell only owns the tab strip
//! and the omnibox; the page itself is rendered by the platform WebView and
//! receives native mouse, keyboard, IME, scroll and clipboard input.
//!
//! The agent drives the same WebView through `eval_with_callback`, so the user
//! and the model always share one page state. There is no screencast, no JPEG
//! pipeline and no coordinate mapping.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};
use mixd_browser::{
    ArtifactKind, BrowserAction, BrowserArtifact, BrowserCall, BrowserHost, BrowserResponse,
    WaitCondition, MAX_DOM_CHARS, MAX_EXTRACT_ITEMS, MAX_SCREENSHOT_BYTES, MAX_SNAPSHOT_NODES,
};
use serde_json::{json, Value};
use tauri::webview::WebviewBuilder;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Position, Rect, Size, Webview,
    WebviewUrl, Wry,
};
use tokio::sync::{oneshot, Mutex};

use super::js;
use super::screenshot;

const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);
const EVAL_TIMEOUT: Duration = Duration::from_secs(15);
const WAIT_POLL: Duration = Duration::from_millis(200);
const MAX_ARTIFACTS: usize = 500;

type AppWebview = Webview<Wry>;

#[derive(Clone, Copy, Debug)]
pub struct SurfaceBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
}

impl Default for SurfaceBounds {
    fn default() -> Self {
        Self {
            // Off-screen and tiny until the React surface reports its real
            // rectangle. If `hide()` is a no-op on a platform, the page must
            // not cover the app shell.
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 240.0,
            visible: false,
        }
    }
}

struct BrowserTab {
    id: String,
    label: String,
    webview: AppWebview,
    title: Option<String>,
}

#[derive(Default)]
struct WorkspaceTabs {
    tabs: Vec<BrowserTab>,
    active: Option<String>,
    next_id: u64,
}

impl WorkspaceTabs {
    fn active_tab(&self) -> Option<&BrowserTab> {
        let id = self.active.as_deref()?;
        self.tabs.iter().find(|tab| tab.id == id)
    }

    fn tab_mut(&mut self, id: &str) -> Option<&mut BrowserTab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }
}

pub struct WebviewBrowserHost {
    app: AppHandle,
    app_data: PathBuf,
    allow_local: Arc<AtomicBool>,
    workspaces: Mutex<HashMap<uuid::Uuid, WorkspaceTabs>>,
    surface: Mutex<HashMap<uuid::Uuid, SurfaceBounds>>,
    workspace_locks: Mutex<HashMap<uuid::Uuid, Arc<Mutex<()>>>>,
    artifacts: Mutex<Vec<BrowserArtifact>>,
    artifacts_path: PathBuf,
}

impl WebviewBrowserHost {
    pub fn new(app: AppHandle, app_data: impl AsRef<Path>) -> Self {
        let app_data = app_data.as_ref().to_path_buf();
        let artifacts_path = app_data.join("artifacts").join("index.json");
        let artifacts = std::fs::read_to_string(&artifacts_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<BrowserArtifact>>(&raw).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|artifact| Path::new(&artifact.path).is_file())
            .collect();
        Self {
            app,
            app_data,
            allow_local: Arc::new(AtomicBool::new(false)),
            workspaces: Mutex::new(HashMap::new()),
            surface: Mutex::new(HashMap::new()),
            workspace_locks: Mutex::new(HashMap::new()),
            artifacts: Mutex::new(artifacts),
            artifacts_path,
        }
    }

    pub async fn set_surface(&self, workspace_id: uuid::Uuid, bounds: SurfaceBounds) {
        {
            let mut surfaces = self.surface.lock().await;
            if bounds.visible {
                // Only one workspace's native view may be on screen. Without
                // this, switching threads would leave the previous workspace's
                // child WebView floating above the new panel.
                for (id, other) in surfaces.iter_mut() {
                    if *id != workspace_id {
                        other.visible = false;
                    }
                }
            }
            surfaces.insert(workspace_id, bounds);
        }
        let workspaces = self.workspaces.lock().await;
        for (id, state) in workspaces.iter() {
            let is_current = *id == workspace_id;
            let active = state.active.as_deref();
            for tab in &state.tabs {
                if is_current && active == Some(tab.id.as_str()) {
                    let _ = tab.webview.set_bounds(rect(bounds));
                    if bounds.visible {
                        let _ = tab.webview.show();
                    } else {
                        let _ = tab.webview.hide();
                    }
                } else {
                    let _ = tab.webview.hide();
                }
            }
        }
    }

    pub async fn clear_data(&self, workspace_id: uuid::Uuid) -> anyhow::Result<()> {
        let _guard = self.workspace_guard(workspace_id).await;
        let mut workspaces = self.workspaces.lock().await;
        if let Some(state) = workspaces.remove(&workspace_id) {
            for tab in state.tabs {
                let _ = tab.webview.clear_all_browsing_data();
                let _ = tab.webview.close();
            }
        }
        let _ = tokio::fs::remove_dir_all(self.profile_dir(workspace_id)).await;
        Ok(())
    }

    pub async fn artifacts(&self) -> Vec<BrowserArtifact> {
        self.artifacts.lock().await.clone()
    }

    pub async fn stop_loading(&self, workspace_id: uuid::Uuid) -> anyhow::Result<()> {
        let _guard = self.workspace_guard(workspace_id).await;
        let webview = self.active_webview(workspace_id).await?;
        let _ = webview.eval(js::stop());
        Ok(())
    }

    async fn workspace_guard(&self, workspace_id: uuid::Uuid) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.workspace_locks.lock().await;
            locks
                .entry(workspace_id)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }

    fn profile_dir(&self, workspace_id: uuid::Uuid) -> PathBuf {
        self.app_data
            .join("webview-profiles")
            .join(workspace_id.to_string())
    }

    fn main_window(&self) -> anyhow::Result<tauri::Window<Wry>> {
        self.app
            .get_window("main")
            .ok_or_else(|| anyhow!("main window is not available"))
    }

    async fn create_tab(
        &self,
        workspace_id: uuid::Uuid,
        state: &mut WorkspaceTabs,
        url: &str,
        bounds: SurfaceBounds,
    ) -> anyhow::Result<()> {
        state.next_id += 1;
        let id = format!("t{}", state.next_id);
        let label = format!(
            "browser-{}-{}",
            &workspace_id.to_string()[..8],
            uuid::Uuid::new_v4()
        );
        let allow_local = self.allow_local.clone();
        let app_page = self.app.clone();
        let app_title = self.app.clone();
        let event_workspace = workspace_id.to_string();
        let event_workspace_title = event_workspace.clone();
        let profile_dir = self.profile_dir(workspace_id);
        let _ = tokio::fs::create_dir_all(&profile_dir).await;

        let parsed = url::Url::parse(url).with_context(|| format!("invalid browser URL: {url}"))?;
        let mut builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(parsed))
            .data_directory(profile_dir)
            .initialization_script(js::egress_guard(allow_local.load(Ordering::Relaxed)))
            .on_navigation(move |url| {
                if url.scheme() == "about" {
                    // `about:srcdoc` is used by same-document iframes; it
                    // never leaves the process and must not be confused with
                    // a top-level `about:` navigation.
                    return matches!(url.as_str(), "about:blank" | "about:srcdoc");
                }
                if url.scheme() == "blob" {
                    return true;
                }
                mixd_browser::reject_private_navigate_url_with(
                    url.as_str(),
                    allow_local.load(Ordering::Relaxed),
                )
                .is_ok()
            })
            .on_page_load(move |_webview, _payload| {
                let _ = app_page.emit(
                    "browser://state",
                    json!({ "workspace_id": event_workspace }),
                );
            })
            .on_document_title_changed(move |_webview, _title| {
                let _ = app_title.emit(
                    "browser://state",
                    json!({ "workspace_id": event_workspace_title }),
                );
            })
            .on_permission_request(|_webview, _kind| tauri::webview::PermissionResponse::Deny)
            .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::<Wry>::Deny);

        // `data_directory` gives Windows/Linux a per-workspace profile. On
        // macOS WKWebView currently shares the app data store (documented).
        builder = builder.incognito(false);

        let window = self.main_window()?;
        let webview = window.add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)),
        )?;
        if !bounds.visible {
            let _ = webview.hide();
        }

        for tab in &state.tabs {
            let _ = tab.webview.hide();
        }
        state.tabs.push(BrowserTab {
            id: id.clone(),
            label,
            webview,
            title: None,
        });
        state.active = Some(id);
        Ok(())
    }

    async fn ensure_active_tab(
        &self,
        workspace_id: uuid::Uuid,
        url: Option<&str>,
    ) -> anyhow::Result<(String, AppWebview)> {
        let bounds = self
            .surface
            .lock()
            .await
            .get(&workspace_id)
            .copied()
            .unwrap_or_default();
        let mut workspaces = self.workspaces.lock().await;
        let state = workspaces.entry(workspace_id).or_default();
        if state.active_tab().is_none() {
            self.create_tab(workspace_id, state, url.unwrap_or("about:blank"), bounds)
                .await?;
        }
        let tab = state
            .active_tab()
            .ok_or_else(|| anyhow!("could not create a browser tab"))?;
        Ok((tab.id.clone(), tab.webview.clone()))
    }

    async fn active_webview(&self, workspace_id: uuid::Uuid) -> anyhow::Result<AppWebview> {
        let workspaces = self.workspaces.lock().await;
        workspaces
            .get(&workspace_id)
            .and_then(WorkspaceTabs::active_tab)
            .map(|tab| tab.webview.clone())
            .ok_or_else(|| anyhow!("no active browser tab"))
    }

    async fn eval_value(
        &self,
        webview: &AppWebview,
        script: String,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let (tx, rx) = oneshot::channel::<String>();
        let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
        webview.eval_with_callback(script, move |raw| {
            if let Some(tx) = tx.lock().ok().and_then(|mut slot| slot.take()) {
                let _ = tx.send(raw);
            }
        })?;
        let raw = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| anyhow!("browser script timed out"))??;
        js::parse_result(&raw)
    }

    async fn eval_action(&self, webview: &AppWebview, script: String) -> anyhow::Result<Value> {
        self.eval_value(webview, script, EVAL_TIMEOUT).await
    }

    async fn wait_ready(&self, webview: &AppWebview, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(value) = self
                .eval_value(webview, js::info(), Duration::from_secs(2))
                .await
            {
                if value.get("readyState").and_then(Value::as_str) == Some("complete") {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn page_info(&self, webview: &AppWebview) -> Value {
        let info = self
            .eval_value(webview, js::info(), Duration::from_secs(3))
            .await
            .unwrap_or(Value::Null);
        // Do not call `Webview::url()` here. wry's macOS getter unwraps
        // `WKWebView.URL()`, which is nil between `add_child` and the first
        // navigation; reading `location.href` in the page is safe and returns
        // `about:blank` for a fresh tab.
        let url = info.get("url").and_then(Value::as_str).map(str::to_string);
        json!({
            "title": info.get("title").cloned().unwrap_or(Value::Null),
            "url": url,
            "canGoBack": info.get("canGoBack").cloned().unwrap_or(Value::Bool(false)),
            "canGoForward": info.get("canGoForward").cloned().unwrap_or(Value::Bool(false)),
        })
    }

    async fn list_tabs(&self, workspace_id: uuid::Uuid) -> Vec<Value> {
        let workspaces = self.workspaces.lock().await;
        let Some(state) = workspaces.get(&workspace_id) else {
            return Vec::new();
        };
        let active = state.active.as_deref();
        let mut out = Vec::with_capacity(state.tabs.len());
        for tab in &state.tabs {
            let info = self.page_info(&tab.webview).await;
            out.push(json!({
                "id": tab.id,
                "url": info.get("url").cloned().unwrap_or(Value::Null),
                "title": info.get("title").cloned().unwrap_or(tab.title.clone().map(Value::String).unwrap_or(Value::Null)),
                "active": active == Some(tab.id.as_str()),
                "label": tab.label,
            }));
        }
        out
    }

    async fn active_info(&self, workspace_id: uuid::Uuid) -> anyhow::Result<Value> {
        let webview = self.active_webview(workspace_id).await?;
        let info = self.page_info(&webview).await;
        Ok(info)
    }

    async fn save_artifact(
        &self,
        workspace_id: uuid::Uuid,
        png: &[u8],
        full_page: bool,
    ) -> anyhow::Result<String> {
        if png.len() > MAX_SCREENSHOT_BYTES {
            anyhow::bail!("screenshot exceeds {} bytes", MAX_SCREENSHOT_BYTES);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let dir = self
            .app_data
            .join("artifacts")
            .join(workspace_id.to_string());
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(format!("{id}.png"));
        tokio::fs::write(&path, png).await?;
        let mut artifacts = self.artifacts.lock().await;
        artifacts.push(BrowserArtifact {
            id: id.clone(),
            run_id: workspace_id,
            kind: ArtifactKind::Screenshot,
            mime_type: "image/png".into(),
            path: path.to_string_lossy().to_string(),
        });
        if artifacts.len() > MAX_ARTIFACTS {
            let excess = artifacts.len() - MAX_ARTIFACTS;
            for artifact in artifacts.drain(..excess) {
                let _ = tokio::fs::remove_file(&artifact.path).await;
            }
        }
        let serialized = serde_json::to_vec_pretty(&*artifacts)?;
        if let Some(parent) = self.artifacts_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&self.artifacts_path, serialized).await?;
        let _ = full_page;
        Ok(id)
    }

    async fn wait_for(
        &self,
        webview: &AppWebview,
        condition: WaitCondition,
        timeout_ms: u64,
    ) -> anyhow::Result<BrowserResponse> {
        if let WaitCondition::Url { url } = &condition {
            mixd_browser::reject_private_navigate_url_with(url, self.allows_local())?;
            mixd_browser::reject_private_navigate_url_resolved_with(url, self.allows_local())
                .await?;
        }
        let timeout_ms = timeout_ms.min(60_000);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let ready = match &condition {
                WaitCondition::Selector { selector } => self
                    .eval_value(
                        webview,
                        js::selector_probe(selector),
                        Duration::from_secs(3),
                    )
                    .await
                    .ok()
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                WaitCondition::Url { url } => self
                    .eval_value(webview, js::url_probe(url), Duration::from_secs(3))
                    .await
                    .ok()
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                WaitCondition::Text { text } => self
                    .eval_value(webview, js::text_probe(text), Duration::from_secs(3))
                    .await
                    .ok()
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                WaitCondition::Load => self
                    .eval_value(webview, js::load_probe(), Duration::from_secs(3))
                    .await
                    .ok()
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                WaitCondition::Function { script } => {
                    if let Err(err) = mixd_browser::validate_evaluate_script(script) {
                        return Ok(BrowserResponse::err(format!("{err:#}")));
                    }
                    self.eval_value(webview, js::function_probe(script), Duration::from_secs(3))
                        .await
                        .ok()
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                }
            };
            if ready {
                return Ok(BrowserResponse::ok(json!({ "ready": true })));
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
        Ok(BrowserResponse::err(format!(
            "condition not met within {timeout_ms} ms"
        )))
    }

    async fn invoke_action(
        &self,
        workspace_id: uuid::Uuid,
        action: BrowserAction,
    ) -> anyhow::Result<BrowserResponse> {
        let allow_local = self.allows_local();
        match action {
            BrowserAction::Navigate { url } => {
                mixd_browser::reject_private_navigate_url_resolved_with(&url, allow_local).await?;
                let (_, webview) = self
                    .ensure_active_tab(workspace_id, Some("about:blank"))
                    .await?;
                let parsed = url::Url::parse(&url)
                    .with_context(|| format!("invalid navigate URL: {url}"))?;
                webview.navigate(parsed)?;
                self.wait_ready(&webview, NAVIGATION_TIMEOUT).await;
                let info = self.page_info(&webview).await;
                Ok(BrowserResponse::ok(json!({
                    "url": info.get("url").cloned().unwrap_or(Value::Null),
                    "title": info.get("title").cloned().unwrap_or(Value::Null),
                })))
            }
            BrowserAction::Click {
                selector,
                reference,
            } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self
                    .eval_action(&webview, js::click(&selector, reference.as_deref()))
                    .await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::ClickAt { x, y } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self.eval_action(&webview, js::click_at(x, y)).await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::Type {
                selector,
                text,
                reference,
                clear,
            } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self
                    .eval_action(
                        &webview,
                        js::type_text(&selector, reference.as_deref(), &text, clear),
                    )
                    .await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::TypeActive { text } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self.eval_action(&webview, js::type_active(&text)).await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::Press { key } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self.eval_action(&webview, js::press(&key)).await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::Hover {
                selector,
                reference,
            } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self
                    .eval_action(&webview, js::hover(&selector, reference.as_deref()))
                    .await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::Select {
                selector,
                value,
                reference,
            } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self
                    .eval_action(
                        &webview,
                        js::select(&selector, reference.as_deref(), &value),
                    )
                    .await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::Check {
                selector,
                checked,
                reference,
            } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self
                    .eval_action(
                        &webview,
                        js::check(&selector, reference.as_deref(), checked),
                    )
                    .await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::Scroll { x, y } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self.eval_action(&webview, js::scroll(x, y)).await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::Dom { selector } => {
                let webview = self.active_webview(workspace_id).await?;
                let value = self
                    .eval_value(&webview, js::dom_html(selector.as_deref()), EVAL_TIMEOUT)
                    .await?;
                let html = value
                    .get("html")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let capped: String = html.chars().take(MAX_DOM_CHARS).collect();
                Ok(BrowserResponse::ok(json!(capped)))
            }
            BrowserAction::Extract { selector, limit } => {
                let webview = self.active_webview(workspace_id).await?;
                let limit = limit.clamp(1, MAX_EXTRACT_ITEMS);
                let value = self
                    .eval_value(&webview, js::extract(&selector, limit), EVAL_TIMEOUT)
                    .await?;
                let items = value
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .take(limit)
                    .collect::<Vec<_>>();
                Ok(BrowserResponse::ok(json!({
                    "count": items.len(),
                    "items": items,
                })))
            }
            BrowserAction::Snapshot { max_nodes } => {
                let webview = self.active_webview(workspace_id).await?;
                let max_nodes = max_nodes.clamp(1, MAX_SNAPSHOT_NODES);
                let value = self
                    .eval_value(&webview, js::snapshot(max_nodes), EVAL_TIMEOUT)
                    .await?;
                let nodes = value
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .take(max_nodes)
                    .collect::<Vec<_>>();
                Ok(BrowserResponse::ok(json!({
                    "count": nodes.len(),
                    "nodes": nodes,
                })))
            }
            BrowserAction::Screenshot { full_page } => {
                let webview = self.active_webview(workspace_id).await?;
                let png = match screenshot::capture(&webview).await {
                    Ok(png) => png,
                    Err(err) => return Ok(BrowserResponse::err(format!("{err:#}"))),
                };
                let id = self
                    .save_artifact(workspace_id, &png, full_page)
                    .await
                    .map_err(|err| anyhow!("saving screenshot failed: {err:#}"))?;
                Ok(BrowserResponse {
                    ok: true,
                    payload: json!({ "bytes": png.len(), "full_page": full_page }),
                    artifact_id: Some(id),
                    error: None,
                })
            }
            BrowserAction::Evaluate { script } => {
                mixd_browser::validate_evaluate_script(&script)?;
                let webview = self.active_webview(workspace_id).await?;
                let value = self
                    .eval_value(&webview, js::evaluate(&script), EVAL_TIMEOUT)
                    .await?;
                Ok(BrowserResponse::ok(value))
            }
            BrowserAction::WaitFor {
                condition,
                timeout_ms,
            } => {
                let webview = self.active_webview(workspace_id).await?;
                self.wait_for(&webview, condition, timeout_ms).await
            }
            BrowserAction::Info => {
                let _ = self
                    .ensure_active_tab(workspace_id, Some("about:blank"))
                    .await?;
                let info = self.active_info(workspace_id).await?;
                let pages = self.list_tabs(workspace_id).await;
                let active = self
                    .workspaces
                    .lock()
                    .await
                    .get(&workspace_id)
                    .and_then(|state| state.active.as_deref())
                    .unwrap_or_default()
                    .to_string();
                Ok(BrowserResponse::ok(json!({
                    "pages": pages,
                    "active": {
                        "id": active,
                        "url": info.get("url").cloned().unwrap_or(Value::Null),
                        "title": info.get("title").cloned().unwrap_or(Value::Null),
                    },
                    "can_go_back": info.get("canGoBack").cloned().unwrap_or(Value::Bool(false)),
                    "can_go_forward": info.get("canGoForward").cloned().unwrap_or(Value::Bool(false)),
                })))
            }
            BrowserAction::Reload => {
                let webview = self.active_webview(workspace_id).await?;
                let _ = self.eval_action(&webview, js::reload()).await;
                self.wait_ready(&webview, NAVIGATION_TIMEOUT).await;
                let info = self.page_info(&webview).await;
                Ok(BrowserResponse::ok(json!({
                    "url": info.get("url").cloned().unwrap_or(Value::Null),
                    "title": info.get("title").cloned().unwrap_or(Value::Null),
                })))
            }
            BrowserAction::Back => {
                let webview = self.active_webview(workspace_id).await?;
                let _ = self.eval_action(&webview, js::back()).await;
                tokio::time::sleep(Duration::from_millis(300)).await;
                let info = self.page_info(&webview).await;
                Ok(BrowserResponse::ok(json!({
                    "url": info.get("url").cloned().unwrap_or(Value::Null),
                    "title": info.get("title").cloned().unwrap_or(Value::Null),
                    "direction": "back",
                })))
            }
            BrowserAction::Forward => {
                let webview = self.active_webview(workspace_id).await?;
                let _ = self.eval_action(&webview, js::forward()).await;
                tokio::time::sleep(Duration::from_millis(300)).await;
                let info = self.page_info(&webview).await;
                Ok(BrowserResponse::ok(json!({
                    "url": info.get("url").cloned().unwrap_or(Value::Null),
                    "title": info.get("title").cloned().unwrap_or(Value::Null),
                    "direction": "forward",
                })))
            }
            BrowserAction::Tabs => {
                let pages = self.list_tabs(workspace_id).await;
                Ok(BrowserResponse::ok(json!({
                    "count": pages.len(),
                    "tabs": pages,
                })))
            }
            BrowserAction::NewTab { url } => {
                let url = url.unwrap_or_else(|| "about:blank".to_string());
                if url != "about:blank" {
                    mixd_browser::reject_private_navigate_url_resolved_with(&url, allow_local)
                        .await?;
                }
                let bounds = self
                    .surface
                    .lock()
                    .await
                    .get(&workspace_id)
                    .copied()
                    .unwrap_or_default();
                let (tab_id, webview) = {
                    let mut workspaces = self.workspaces.lock().await;
                    let state = workspaces.entry(workspace_id).or_default();
                    self.create_tab(workspace_id, state, &url, bounds).await?;
                    let tab = state
                        .active_tab()
                        .ok_or_else(|| anyhow!("could not create a browser tab"))?;
                    (tab.id.clone(), tab.webview.clone())
                };
                self.wait_ready(&webview, NAVIGATION_TIMEOUT).await;
                let info = self.page_info(&webview).await;
                Ok(BrowserResponse::ok(json!({
                    "id": tab_id,
                    "url": info.get("url").cloned().unwrap_or(Value::Null),
                    "title": info.get("title").cloned().unwrap_or(Value::Null),
                })))
            }
            BrowserAction::SwitchTab { tab_id } => {
                let bounds = self
                    .surface
                    .lock()
                    .await
                    .get(&workspace_id)
                    .copied()
                    .unwrap_or_default();
                let mut workspaces = self.workspaces.lock().await;
                let state = workspaces
                    .get_mut(&workspace_id)
                    .ok_or_else(|| anyhow!("tab_not_found: {tab_id}"))?;
                if state.tab_mut(&tab_id).is_none() {
                    return Ok(BrowserResponse::err(format!("tab_not_found: {tab_id}")));
                }
                state.active = Some(tab_id.clone());
                for tab in &state.tabs {
                    if tab.id == tab_id {
                        let _ = tab.webview.set_bounds(rect(bounds));
                        if bounds.visible {
                            let _ = tab.webview.show();
                        }
                    } else {
                        let _ = tab.webview.hide();
                    }
                }
                let tab = state
                    .tab_mut(&tab_id)
                    .ok_or_else(|| anyhow!("tab_not_found: {tab_id}"))?;
                let info = self.page_info(&tab.webview).await;
                Ok(BrowserResponse::ok(json!({
                    "id": tab.id,
                    "url": info.get("url").cloned().unwrap_or(Value::Null),
                    "title": info.get("title").cloned().unwrap_or(Value::Null),
                })))
            }
            BrowserAction::CloseTab { tab_id } => {
                let bounds = self
                    .surface
                    .lock()
                    .await
                    .get(&workspace_id)
                    .copied()
                    .unwrap_or_default();
                let mut workspaces = self.workspaces.lock().await;
                let state = workspaces
                    .get_mut(&workspace_id)
                    .ok_or_else(|| anyhow!("tab_not_found"))?;
                let id = tab_id
                    .or_else(|| state.active.clone())
                    .ok_or_else(|| anyhow!("tab_not_found"))?;
                let Some(index) = state.tabs.iter().position(|tab| tab.id == id) else {
                    return Ok(BrowserResponse::err(format!("tab_not_found: {id}")));
                };
                let tab = state.tabs.remove(index);
                let _ = tab.webview.close();
                if state.active.as_deref() == Some(id.as_str()) {
                    state.active = state.tabs.first().map(|tab| tab.id.clone());
                }
                if let Some(active) = state.active.clone() {
                    for tab in &state.tabs {
                        if tab.id == active {
                            let _ = tab.webview.set_bounds(rect(bounds));
                            if bounds.visible {
                                let _ = tab.webview.show();
                            }
                        } else {
                            let _ = tab.webview.hide();
                        }
                    }
                }
                Ok(BrowserResponse::ok(json!({ "closed": id })))
            }
            BrowserAction::Close => {
                let mut workspaces = self.workspaces.lock().await;
                if let Some(state) = workspaces.remove(&workspace_id) {
                    for tab in state.tabs {
                        let _ = tab.webview.close();
                    }
                }
                Ok(BrowserResponse::ok(json!({ "closed": true })))
            }
        }
    }
}

fn rect(bounds: SurfaceBounds) -> Rect {
    Rect {
        position: Position::Logical(LogicalPosition::new(bounds.x, bounds.y)),
        size: Size::Logical(LogicalSize::new(
            bounds.width.max(1.0),
            bounds.height.max(1.0),
        )),
    }
}

#[async_trait::async_trait]
impl BrowserHost for WebviewBrowserHost {
    async fn invoke(&self, call: BrowserCall) -> anyhow::Result<BrowserResponse> {
        let _guard = self.workspace_guard(call.workspace_id).await;
        match self.invoke_action(call.workspace_id, call.action).await {
            Ok(response) => Ok(response),
            Err(err) => {
                tracing::warn!(workspace = %call.workspace_id, %err, "webview browser call failed");
                Ok(BrowserResponse::err(format!("{err:#}")))
            }
        }
    }

    fn allows_local(&self) -> bool {
        self.allow_local.load(Ordering::Relaxed)
    }

    async fn set_allow_local(&self, value: bool) {
        self.allow_local.store(value, Ordering::Relaxed);
        let workspaces = self.workspaces.lock().await;
        for state in workspaces.values() {
            for tab in &state.tabs {
                let _ = tab.webview.eval(js::egress_guard(value));
            }
        }
    }
}
