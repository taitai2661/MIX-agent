//! `chromiumoxide`-backed Chromium implementation of [`BrowserTool`].
//!
//! The Agent never sees this type; the [`crate::operator::BrowserOperator`]
//! owns the lifecycle and routes calls here.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, CaptureScreenshotParams, Viewport,
};
use chromiumoxide::cdp::browser_protocol::target::TargetId;
use chromiumoxide::layout::Point;
use chromiumoxide::page::Page;
use futures_util::{FutureExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

use crate::interface::{
    BrowserAction, BrowserCall, BrowserResponse, BrowserTool, WaitCondition, MAX_DOM_CHARS,
    MAX_EXTRACT_ITEMS, MAX_SCREENSHOT_BYTES, MAX_SNAPSHOT_NODES,
};
/// How long a Chromium launch may take before the call is failed. Launching is
/// unbounded otherwise, which would hold the launch lock forever.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);
/// A navigation that never settles must not park the whole workspace forever.
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);
/// History navigation and reload are best-effort: a page that does not emit a
/// navigation event still needs to return so the agent can inspect it.
const HISTORY_WAIT: Duration = Duration::from_secs(5);

pub struct ChromiumBackend {
    executable: Option<PathBuf>,
    headless: bool,
    user_data_dir: Option<PathBuf>,
    browser: Arc<Mutex<Option<Browser>>>,
    /// The CDP handler is a `Stream`, not a `Future`: it must be polled for
    /// any command (including `GetPages`) to receive a response. Keeping the
    /// `Handler` value in a field without spawning it makes every browser call
    /// hang forever.
    handler_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Serialises launches without holding `browser` across them.
    launch_lock: Mutex<()>,
    /// The tab every action runs against. `chromiumoxide::Browser::pages()`
    /// is backed by a `HashMap`, so "first page" is not a stable notion; the
    /// operator remembers the tab explicitly instead.
    active_page: Mutex<Option<Page>>,
    artifact_dir: PathBuf,
    /// Workspace-scoped opt-in for loopback / RFC1918 navigation (local dev
    /// servers). Link-local metadata stays blocked regardless.
    allow_local: AtomicBool,
}

impl ChromiumBackend {
    pub fn new(
        executable: Option<PathBuf>,
        user_data_dir: Option<PathBuf>,
        artifact_dir: PathBuf,
        headless: bool,
    ) -> Self {
        Self {
            executable,
            headless,
            user_data_dir,
            browser: Arc::new(Mutex::new(None)),
            handler_task: Arc::new(Mutex::new(None)),
            launch_lock: Mutex::new(()),
            active_page: Mutex::new(None),
            artifact_dir,
            allow_local: AtomicBool::new(false),
        }
    }

    pub fn set_allow_local(&self, value: bool) {
        self.allow_local.store(value, Ordering::Relaxed);
    }

    pub fn allows_local(&self) -> bool {
        self.allow_local.load(Ordering::Relaxed)
    }

    async fn ensure_browser(&self) -> anyhow::Result<()> {
        if self.browser.lock().await.is_some() {
            return Ok(());
        }
        // The launch lock serialises concurrent launchers; the `browser` mutex
        // itself is never held across `Browser::launch`, so a hung launch cannot
        // block `Close` or any other action for its whole duration.
        let _launch = self.launch_lock.lock().await;
        if self.browser.lock().await.is_some() {
            return Ok(());
        }
        let mut builder = BrowserConfig::builder()
            .window_size(1280, 900)
            // Screenshot capture can otherwise wait for a compositor frame
            // that never reaches the headless target. These flags force the
            // frame to be produced before `Page.captureScreenshot` is answered.
            .arg("--disable-gpu")
            .arg("--run-all-compositor-stages-before-draw")
            .arg("--disable-new-content-rendering-timeout");
        if let Some(exe) = &self.executable {
            builder = builder.chrome_executable(exe);
        }
        if let Some(dir) = &self.user_data_dir {
            builder = builder.user_data_dir(dir);
        }
        if self.headless {
            // Chromium 132+ removed the old headless mode. The legacy flag
            // still starts the browser, but screenshot/compositor commands can
            // never settle; the new headless mode supports them.
            builder = builder.headless_mode(chromiumoxide::browser::HeadlessMode::New);
        } else {
            builder = builder.with_head();
        }
        let config = match builder.build() {
            Ok(config) => config,
            Err(err) => {
                tracing::error!(%err, "building the Chromium configuration failed");
                return Err(anyhow::anyhow!("{}", err));
            }
        };
        let (browser, handler) = match timeout(LAUNCH_TIMEOUT, Browser::launch(config)).await {
            Ok(Ok(launched)) => launched,
            Ok(Err(err)) => {
                // Launching is the one Browser failure the app cannot recover
                // from on its own, so it has to be visible in the log.
                tracing::error!(
                    executable = ?self.executable,
                    %err,
                    "failed to launch Chromium"
                );
                return Err(anyhow::anyhow!("failed to launch Chromium: {}", err));
            }
            Err(_) => {
                tracing::error!(
                    executable = ?self.executable,
                    "timed out launching Chromium"
                );
                return Err(anyhow::anyhow!(
                    "timed out launching Chromium after {:?}",
                    LAUNCH_TIMEOUT
                ));
            }
        };
        // Drive the CDP stream. Without this task the sender accepts commands
        // but no response is ever read, so `pages()`, `goto()` and every other
        // call waits forever.
        let task = tokio::spawn(async move {
            let mut handler = handler;
            while let Some(item) = handler.next().await {
                if let Err(err) = item {
                    tracing::warn!(%err, "Chromium CDP handler stopped");
                    break;
                }
            }
        });
        *self.handler_task.lock().await = Some(task);
        *self.browser.lock().await = Some(browser);
        tracing::info!(executable = ?self.executable, "Chromium launched");
        Ok(())
    }

    /// Run a closure against the live browser, launching it on demand.
    async fn with_browser<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        F: for<'a> FnOnce(&'a Browser) -> futures_util::future::BoxFuture<'a, anyhow::Result<T>>,
    {
        self.ensure_browser().await?;
        let guard = self.browser.lock().await;
        let browser = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("browser is not running"))?;
        f(browser).await
    }

    /// The tab actions run against. Reuses an existing page (including the
    /// initial `about:blank`) instead of opening a fresh tab on every
    /// navigation.
    async fn current_page(&self) -> anyhow::Result<Page> {
        self.ensure_browser().await?;
        if let Some(page) = self.active_page.lock().await.clone() {
            return Ok(page);
        }
        let page = self
            .with_browser(|b| {
                Box::pin(async move {
                    let mut pages = b
                        .pages()
                        .await
                        .map_err(|err| anyhow::anyhow!("listing pages failed: {err}"))?;
                    if let Some(page) = pages.pop() {
                        Ok(page)
                    } else {
                        b.new_page("about:blank")
                            .await
                            .map_err(|err| anyhow::anyhow!("creating a page failed: {err}"))
                    }
                })
            })
            .await?;
        *self.active_page.lock().await = Some(page.clone());
        Ok(page)
    }

    async fn set_active(&self, page: &Page) {
        *self.active_page.lock().await = Some(page.clone());
    }

    async fn page_by_id(&self, tab_id: &str) -> anyhow::Result<Page> {
        let page = self
            .with_browser(|b| {
                let id = tab_id.to_string();
                Box::pin(async move {
                    b.get_page(TargetId::new(id))
                        .await
                        .map_err(|err| anyhow::anyhow!("tab not found: {err}"))
                })
            })
            .await?;
        Ok(page)
    }

    async fn tab_json(&self, page: &Page, active: &str) -> Value {
        json!({
            "id": page.target_id().inner(),
            "url": page.url().await.ok().flatten(),
            "title": page.get_title().await.ok().flatten(),
            "active": page.target_id().inner() == active,
        })
    }

    async fn list_tabs(&self, active: &str) -> anyhow::Result<Vec<Value>> {
        let pages = self
            .with_browser(|b| {
                Box::pin(async move {
                    b.pages()
                        .await
                        .map_err(|err| anyhow::anyhow!("listing pages failed: {err}"))
                })
            })
            .await?;
        let mut out = Vec::with_capacity(pages.len());
        for page in pages {
            out.push(self.tab_json(&page, active).await);
        }
        Ok(out)
    }

    async fn navigate(&self, url: &str, allow_local: bool) -> anyhow::Result<BrowserResponse> {
        if let Err(err) = crate::interface::reject_private_navigate_url_with(url, allow_local) {
            return Ok(BrowserResponse::err(format!("navigation blocked: {err:#}")));
        }
        if let Err(err) =
            crate::interface::reject_private_navigate_url_resolved_with(url, allow_local).await
        {
            return Ok(BrowserResponse::err(format!("navigation blocked: {err:#}")));
        }
        let page = self.current_page().await?;
        page.goto(url).await?;
        let _ = timeout(NAVIGATION_TIMEOUT, page.wait_for_navigation()).await;
        let final_url = page
            .url()
            .await
            .unwrap_or(None)
            .unwrap_or_else(|| url.to_string());
        let title = page.get_title().await.unwrap_or(None);
        self.set_active(&page).await;
        Ok(BrowserResponse::ok(
            json!({ "url": final_url, "title": title }),
        ))
    }

    async fn select_element(
        &self,
        selector: &str,
        reference: Option<&str>,
    ) -> anyhow::Result<chromiumoxide::element::Element> {
        let selector = resolve_selector(selector, reference)?;
        let page = self.current_page().await?;
        page.find_element(selector)
            .await
            .map_err(|err| anyhow::anyhow!("selector_not_found: {err}"))
    }

    /// Scroll an element to the centre before asking for its click point.
    ///
    /// `Element::scroll_into_view()` waits for an `IntersectionObserver` that
    /// can never fire on some headless pages, so this uses a plain in-page
    /// `scrollIntoView` with a hard timeout. Best-effort: an element that is
    /// already in view needs nothing, and a page that refuses to scroll still
    /// gets its content-quad click.
    async fn bring_into_view(&self, element: &chromiumoxide::element::Element) {
        let scrolled = timeout(
            Duration::from_secs(2),
            element.call_js_fn(
                "function() {
                   this.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
                   return true;
                 }",
                false,
            ),
        )
        .await;
        if scrolled.is_err() {
            tracing::debug!("scrollIntoView timed out; clicking the current quad");
        }
    }

    /// Capture through the CDP command directly instead of
    /// `Page::screenshot()`, whose unconditional `Page.activate` can hang on
    /// a headless target. This path also lets full-page use a deterministic
    /// clip instead of resizing the device metrics and restoring them.
    ///
    /// `jpeg_quality` switches the encoding: PNG for artifacts, JPEG with
    /// `optimize_for_speed` for the live-view poller.
    async fn screenshot_bytes(
        &self,
        full_page: bool,
        jpeg_quality: Option<i64>,
    ) -> anyhow::Result<Vec<u8>> {
        let page = self.current_page().await?;
        // Put the tab in front and wait for one animation frame. Chromium can
        // otherwise accept `Page.captureScreenshot` before the compositor has
        // produced a frame, and the command sits until the CDP timeout.
        let _ = timeout(
            Duration::from_secs(2),
            page.evaluate("new Promise(r => requestAnimationFrame(() => r(true)))"),
        )
        .await;
        let mut builder = CaptureScreenshotParams::builder().from_surface(true);
        if let Some(quality) = jpeg_quality {
            builder = builder
                .format(CaptureScreenshotFormat::Jpeg)
                .quality(quality.clamp(1, 100))
                .optimize_for_speed(true);
        } else {
            builder = builder.format(CaptureScreenshotFormat::Png);
        }
        if full_page {
            let metrics = page.layout_metrics().await?;
            builder = builder.clip(Viewport {
                x: 0.,
                y: 0.,
                width: metrics.css_content_size.width,
                height: metrics.css_content_size.height,
                scale: 1.,
            });
        }
        let response = page.execute(builder.build()).await?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(response.result.data)
            .map_err(|err| anyhow::anyhow!("decoding screenshot failed: {err}"))?;
        if bytes.len() > MAX_SCREENSHOT_BYTES {
            anyhow::bail!("screenshot exceeds {} bytes", MAX_SCREENSHOT_BYTES);
        }
        Ok(bytes)
    }

    async fn screenshot(&self, full_page: bool) -> anyhow::Result<BrowserResponse> {
        let png = match self.screenshot_bytes(full_page, None).await {
            Ok(png) => png,
            Err(err) => return Ok(BrowserResponse::err(format!("{err:#}"))),
        };
        let id = uuid::Uuid::new_v4().to_string();
        tokio::fs::create_dir_all(&self.artifact_dir).await?;
        let path = self.artifact_dir.join(format!("{}.png", id));
        tokio::fs::write(&path, &png).await?;
        Ok(BrowserResponse {
            ok: true,
            payload: json!({ "bytes": png.len(), "full_page": full_page }),
            artifact_id: Some(id),
            error: None,
        })
    }

    /// Capture a JPEG frame without writing an artifact. Used by the live-view
    /// poller, where writing one file per frame would grow the disk without
    /// bound.
    ///
    /// JPEG + `optimize_for_speed` keeps the 1.5 s poll cheap enough to leave
    /// on; artifacts keep PNG. The CSS viewport dimensions travel with the
    /// image so the UI can map a click to page coordinates even when the
    /// screenshot's pixel dimensions differ (Retina / device-scale-factor).
    pub async fn capture_frame(&self, full_page: bool) -> anyhow::Result<(Vec<u8>, u32, u32)> {
        let page = self.current_page().await?;
        let metrics = page.layout_metrics().await?;
        let jpeg = self.screenshot_bytes(full_page, Some(70)).await?;
        Ok((
            jpeg,
            metrics.css_layout_viewport.client_width.max(1) as u32,
            metrics.css_layout_viewport.client_height.max(1) as u32,
        ))
    }

    async fn run_js(&self, script: &str) -> anyhow::Result<BrowserResponse> {
        if let Err(err) = crate::interface::validate_evaluate_script(script) {
            return Ok(BrowserResponse::err(format!("script rejected: {err:#}")));
        }
        let page = self.current_page().await?;
        let result = page.evaluate(script).await?;
        let value = result.value().cloned().unwrap_or(Value::Null);
        Ok(BrowserResponse::ok(value))
    }

    async fn wait_for(
        &self,
        condition: WaitCondition,
        timeout_ms: u64,
    ) -> anyhow::Result<BrowserResponse> {
        /// Hard ceiling on a single `wait_for` poll loop.
        const MAX_WAIT_FOR_TIMEOUT_MS: u64 = 60_000;
        if let WaitCondition::Url { url } = &condition {
            if let Err(err) =
                crate::interface::reject_private_navigate_url_with(url, self.allows_local())
            {
                return Ok(BrowserResponse::err(format!(
                    "wait_for target blocked: {err:#}"
                )));
            }
            if let Err(err) = crate::interface::reject_private_navigate_url_resolved_with(
                url,
                self.allows_local(),
            )
            .await
            {
                return Ok(BrowserResponse::err(format!(
                    "wait_for target blocked: {err:#}"
                )));
            }
        }
        let timeout_ms = timeout_ms.min(MAX_WAIT_FOR_TIMEOUT_MS);
        let script = match &condition {
            WaitCondition::Selector { selector } => crate::dom::selector_probe(selector),
            WaitCondition::Url { url } => {
                format!("window.location.href === {}", crate::dom::js_string(url))
            }
            WaitCondition::Text { text } => crate::dom::wait_for_text_script(text),
            WaitCondition::Load => crate::dom::wait_for_load_script(),
            WaitCondition::Function { script } => script.clone(),
        };
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Ok(response) = self.run_js(&script).await {
                if response.payload.as_bool() == Some(true) {
                    return Ok(response);
                }
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(BrowserResponse::err(format!(
            "condition not met within {} ms",
            timeout_ms
        )))
    }

    async fn history(&self, direction: &str) -> anyhow::Result<BrowserResponse> {
        let page = self.current_page().await?;
        page.evaluate(format!("history.{direction}()")).await?;
        let _ = timeout(HISTORY_WAIT, page.wait_for_navigation()).await;
        Ok(BrowserResponse::ok(json!({
            "url": page.url().await.unwrap_or(None),
            "title": page.get_title().await.unwrap_or(None),
            "direction": direction,
        })))
    }
}

/// Prefer a snapshot ref over a CSS selector. A ref is validated as `r<digits>`
/// so it can never smuggle arbitrary CSS/JS into the attribute selector.
fn resolve_selector(selector: &str, reference: Option<&str>) -> anyhow::Result<String> {
    if let Some(reference) = reference {
        let valid = reference
            .strip_prefix('r')
            .map(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false);
        if !valid {
            anyhow::bail!("invalid ref {reference:?}; expected r<number> from snapshot");
        }
        return Ok(format!("[data-mix-ref=\"{reference}\"]"));
    }
    if selector.trim().is_empty() {
        anyhow::bail!("selector or ref is required");
    }
    Ok(selector.to_string())
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max).collect();
    out.push('…');
    out
}

#[async_trait]
impl BrowserTool for ChromiumBackend {
    async fn invoke(&self, call: BrowserCall) -> anyhow::Result<BrowserResponse> {
        let allow_local = self.allows_local();
        match call.action {
            BrowserAction::Navigate { url } => self.navigate(&url, allow_local).await,
            BrowserAction::Click {
                selector,
                reference,
            } => {
                match self.select_element(&selector, reference.as_deref()).await {
                    Ok(element) => {
                        // `Element::click()` scrolls via an IntersectionObserver
                        // first, which can never fire in some headless pages and
                        // then hangs the whole call. Clicking the element's
                        // content-quad centre is a real CDP mouse event without
                        // that observer; scroll explicitly first so an element
                        // below the fold still lands inside the viewport.
                        self.bring_into_view(&element).await;
                        let point = match element.clickable_point().await {
                            Ok(point) => point,
                            Err(err) => {
                                return Ok(BrowserResponse::err(format!(
                                    "selector_not_found or not visible: {err}"
                                )))
                            }
                        };
                        let page = self.current_page().await?;
                        if let Err(err) = page.click(point).await {
                            return Ok(BrowserResponse::err(format!("click failed: {err}")));
                        }
                        Ok(BrowserResponse::ok(json!({
                            "selector": resolve_selector(&selector, reference.as_deref()).ok(),
                            "x": point.x,
                            "y": point.y,
                        })))
                    }
                    Err(err) => Ok(BrowserResponse::err(format!("{err:#}"))),
                }
            }
            BrowserAction::ClickAt { x, y } => {
                let page = self.current_page().await?;
                page.click(Point::new(x as f64, y as f64)).await?;
                Ok(BrowserResponse::ok(json!({ "x": x, "y": y })))
            }
            BrowserAction::Type {
                selector,
                text,
                reference,
                clear,
            } => {
                let element = match self.select_element(&selector, reference.as_deref()).await {
                    Ok(element) => element,
                    Err(err) => return Ok(BrowserResponse::err(format!("{err:#}"))),
                };
                // Focus directly instead of `Element::click()`: the latter
                // waits for an IntersectionObserver that can hang headless.
                if let Err(err) = element
                    .call_js_fn("function() { this.focus(); return true; }", true)
                    .await
                {
                    return Ok(BrowserResponse::err(format!("type focus failed: {err}")));
                }
                if clear {
                    if let Err(err) = element.call_js_fn(crate::dom::CLEAR_FIELD_JS, true).await {
                        return Ok(BrowserResponse::err(format!("clear failed: {err}")));
                    }
                }
                if let Err(err) = element.type_str(&text).await {
                    return Ok(BrowserResponse::err(format!("type failed: {err}")));
                }
                Ok(BrowserResponse::ok(
                    json!({ "typed": text.chars().count() }),
                ))
            }
            BrowserAction::TypeActive { text } => {
                let page = self.current_page().await?;
                let element = match page.find_element(":focus").await {
                    Ok(element) => element,
                    Err(_) => page.find_element("body").await?,
                };
                element.type_str(&text).await?;
                Ok(BrowserResponse::ok(
                    json!({ "typed": text.chars().count() }),
                ))
            }
            BrowserAction::Press { key } => {
                let page = self.current_page().await?;
                let element = match page.find_element(":focus").await {
                    Ok(element) => element,
                    Err(_) => page.find_element("body").await?,
                };
                element.press_key(&key).await?;
                Ok(BrowserResponse::ok(json!({ "key": key })))
            }
            BrowserAction::Hover {
                selector,
                reference,
            } => {
                let element = match self.select_element(&selector, reference.as_deref()).await {
                    Ok(element) => element,
                    Err(err) => return Ok(BrowserResponse::err(format!("{err:#}"))),
                };
                self.bring_into_view(&element).await;
                let point = element.clickable_point().await?;
                let page = self.current_page().await?;
                page.move_mouse(point).await?;
                Ok(BrowserResponse::ok(json!({ "hovered": true })))
            }
            BrowserAction::Select {
                selector,
                value,
                reference,
            } => {
                let element = match self.select_element(&selector, reference.as_deref()).await {
                    Ok(element) => element,
                    Err(err) => return Ok(BrowserResponse::err(format!("{err:#}"))),
                };
                let script = format!(
                    r#"function() {{
                      this.value = {};
                      this.dispatchEvent(new Event('input', {{ bubbles: true }}));
                      this.dispatchEvent(new Event('change', {{ bubbles: true }}));
                      return this.value;
                    }}"#,
                    crate::dom::js_string(&value)
                );
                let result = element.call_js_fn(script, true).await?;
                Ok(BrowserResponse::ok(json!({
                    "value": result.result.value.unwrap_or(Value::Null),
                })))
            }
            BrowserAction::Check {
                selector,
                checked,
                reference,
            } => {
                let element = match self.select_element(&selector, reference.as_deref()).await {
                    Ok(element) => element,
                    Err(err) => return Ok(BrowserResponse::err(format!("{err:#}"))),
                };
                let script = format!(
                    r#"function() {{
                      if (this.checked !== {checked}) this.click();
                      return this.checked;
                    }}"#
                );
                let result = element.call_js_fn(script, true).await?;
                Ok(BrowserResponse::ok(json!({
                    "checked": result.result.value.unwrap_or(Value::Bool(checked)),
                })))
            }
            BrowserAction::Scroll { x, y } => {
                self.run_js(&format!("window.scrollBy({}, {}); window.scrollY", x, y))
                    .await
            }
            BrowserAction::Dom { selector } => {
                let response = self
                    .run_js(&crate::dom::inner_html_script(
                        selector.as_deref(),
                        MAX_DOM_CHARS,
                    ))
                    .await?;
                if let Some(html) = response.payload.as_str() {
                    if html.chars().count() > MAX_DOM_CHARS {
                        return Ok(BrowserResponse::ok(json!(truncate_chars(
                            html,
                            MAX_DOM_CHARS
                        ))));
                    }
                }
                Ok(response)
            }
            BrowserAction::Extract { selector, limit } => {
                let limit = limit.clamp(1, MAX_EXTRACT_ITEMS);
                let response = self
                    .run_js(&crate::dom::extract_script(&selector, limit))
                    .await?;
                let items = response
                    .payload
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
                let max_nodes = max_nodes.clamp(1, MAX_SNAPSHOT_NODES);
                let response = self.run_js(&crate::dom::snapshot_script(max_nodes)).await?;
                let nodes = response
                    .payload
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
            BrowserAction::Screenshot { full_page } => self.screenshot(full_page).await,
            BrowserAction::Evaluate { script } => self.run_js(&script).await,
            BrowserAction::WaitFor {
                condition,
                timeout_ms,
            } => self.wait_for(condition, timeout_ms).await,
            BrowserAction::Info => {
                let page = self.current_page().await?;
                let active = page.target_id().inner().clone();
                let pages = self.list_tabs(&active).await?;
                Ok(BrowserResponse::ok(json!({
                    "pages": pages,
                    "active": {
                        "id": active,
                        "url": page.url().await.unwrap_or(None),
                        "title": page.get_title().await.unwrap_or(None),
                    }
                })))
            }
            BrowserAction::Reload => {
                let page = self.current_page().await?;
                page.reload().await?;
                let _ = timeout(NAVIGATION_TIMEOUT, page.wait_for_navigation()).await;
                Ok(BrowserResponse::ok(json!({
                    "url": page.url().await.unwrap_or(None),
                    "title": page.get_title().await.unwrap_or(None),
                })))
            }
            BrowserAction::Back => self.history("back").await,
            BrowserAction::Forward => self.history("forward").await,
            BrowserAction::Tabs => {
                let page = self.current_page().await?;
                let active = page.target_id().inner().clone();
                let pages = self.list_tabs(&active).await?;
                Ok(BrowserResponse::ok(json!({
                    "count": pages.len(),
                    "tabs": pages,
                })))
            }
            BrowserAction::NewTab { url } => {
                let url = url.unwrap_or_else(|| "about:blank".to_string());
                // Every new tab goes through the same scheme + SSRF gate as
                // `navigate`. Checking only the http(s) prefix let `file:`,
                // `data:` and `javascript:` targets reach Chromium, where a
                // `data:` page runs script outside the `evaluate` gate and a
                // `file:` page can be read back with `dom`/`screenshot`.
                if let Err(err) =
                    crate::interface::reject_private_navigate_url_resolved_with(&url, allow_local)
                        .await
                {
                    return Ok(BrowserResponse::err(format!("navigation blocked: {err:#}")));
                }
                let page = self
                    .with_browser(|b| {
                        let url = url.clone();
                        Box::pin(async move {
                            b.new_page(url)
                                .await
                                .map_err(|err| anyhow::anyhow!("creating a tab failed: {err}"))
                        })
                    })
                    .await?;
                let _ = timeout(NAVIGATION_TIMEOUT, page.wait_for_navigation()).await;
                self.set_active(&page).await;
                Ok(BrowserResponse::ok(json!({
                    "id": page.target_id().inner(),
                    "url": page.url().await.unwrap_or(None),
                    "title": page.get_title().await.unwrap_or(None),
                })))
            }
            BrowserAction::SwitchTab { tab_id } => {
                let page = match self.page_by_id(&tab_id).await {
                    Ok(page) => page,
                    Err(err) => return Ok(BrowserResponse::err(format!("{err:#}"))),
                };
                let _ = page.bring_to_front().await;
                self.set_active(&page).await;
                Ok(BrowserResponse::ok(json!({
                    "id": page.target_id().inner(),
                    "url": page.url().await.unwrap_or(None),
                    "title": page.get_title().await.unwrap_or(None),
                })))
            }
            BrowserAction::CloseTab { tab_id } => {
                let target = match tab_id {
                    Some(id) => match self.page_by_id(&id).await {
                        Ok(page) => page,
                        Err(err) => return Ok(BrowserResponse::err(format!("{err:#}"))),
                    },
                    None => self.current_page().await?,
                };
                let closed_id = target.target_id().inner().clone();
                let active_before = self
                    .active_page
                    .lock()
                    .await
                    .as_ref()
                    .map(|page| page.target_id().inner().clone());
                target.close().await?;
                // Closing a background tab must not yank the active tab out
                // from under the user. Only when the closed tab *was* active
                // pick a replacement, and pick it deterministically instead of
                // trusting `pages()`'s HashMap order.
                if active_before.as_deref() == Some(closed_id.as_str()) || active_before.is_none() {
                    let mut pages = self
                        .with_browser(|b| {
                            Box::pin(async move {
                                b.pages()
                                    .await
                                    .map_err(|err| anyhow::anyhow!("listing pages failed: {err}"))
                            })
                        })
                        .await?;
                    pages.sort_by_key(|page| page.target_id().inner().clone());
                    *self.active_page.lock().await = pages.into_iter().next();
                }
                Ok(BrowserResponse::ok(json!({
                    "closed": closed_id,
                })))
            }
            BrowserAction::Close => {
                let mut guard = self.browser.lock().await;
                if let Some(browser) = guard.as_mut() {
                    let _ = browser.close().await;
                }
                *guard = None;
                if let Some(task) = self.handler_task.lock().await.take() {
                    task.abort();
                }
                *self.active_page.lock().await = None;
                Ok(BrowserResponse::ok(json!({ "closed": true })))
            }
        }
    }
}

/// Keep a helper available for callers that need a ready-made tab list.
pub async fn tab_summary(page: &Page) -> Value {
    json!({
        "id": page.target_id().inner(),
        "url": page.url().await.ok().flatten(),
        "title": page.get_title().await.ok().flatten(),
    })
}

/// Run JavaScript against the active page, applying the same script gate as
/// the `evaluate` action.
pub async fn run_js(backend: &ChromiumBackend, script: &str) -> anyhow::Result<BrowserResponse> {
    backend.run_js(script).await
}

/// Poll a condition until it holds or the timeout expires.
pub async fn wait_for(
    backend: &ChromiumBackend,
    condition: WaitCondition,
    timeout_ms: u64,
) -> anyhow::Result<BrowserResponse> {
    backend.wait_for(condition, timeout_ms).await
}

/// Wait for a navigation event on the active page.
pub async fn wait_for_navigation(backend: &ChromiumBackend) -> anyhow::Result<BrowserResponse> {
    let page = backend.current_page().await?;
    let _ = timeout(NAVIGATION_TIMEOUT, page.wait_for_navigation()).await;
    Ok(BrowserResponse::ok(json!({
        "url": page.url().await.unwrap_or(None),
    })))
}

/// Keep the `FutureExt` import meaningful for callers that chain onto these.
pub fn boxed<F>(fut: F) -> futures_util::future::BoxFuture<'static, anyhow::Result<()>>
where
    F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    fut.boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_are_strictly_validated() {
        assert_eq!(
            resolve_selector("", Some("r12")).unwrap(),
            "[data-mix-ref=\"r12\"]"
        );
        for bad in ["r", "r12x", "../../x", "r-1", ""] {
            assert!(resolve_selector("", Some(bad)).is_err(), "{bad} must fail");
        }
        assert!(resolve_selector("", None).is_err());
        assert_eq!(resolve_selector("#go", None).unwrap(), "#go");
    }

    #[test]
    fn text_truncation_is_character_based() {
        assert_eq!(truncate_chars("abc", 5), "abc");
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
        // Multi-byte input must not be cut mid-codepoint.
        assert_eq!(truncate_chars("あいうえお", 2), "あい…");
    }
}
