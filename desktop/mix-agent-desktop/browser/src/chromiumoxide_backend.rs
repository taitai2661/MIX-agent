//! `chromiumoxide`-backed Chromium implementation of [`BrowserTool`].
//!
//! The Agent never sees this type; the [`crate::operator::BrowserOperator`]
//! owns the lifecycle and routes calls here.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures_util::FutureExt;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

use crate::interface::{BrowserAction, BrowserCall, BrowserResponse, BrowserTool, WaitCondition};
use crate::profile::BrowserProfile;

/// How long a Chromium launch may take before the call is failed. Launching is
/// unbounded otherwise, which would hold the launch lock forever.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);

pub struct ChromiumBackend {
    executable: Option<PathBuf>,
    headless: bool,
    user_data_dir: Option<PathBuf>,
    browser: Arc<Mutex<Option<Browser>>>,
    handler: Arc<Mutex<Option<chromiumoxide::Handler>>>,
    /// Serialises launches without holding `browser` across them.
    launch_lock: Mutex<()>,
    artifact_dir: PathBuf,
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
            handler: Arc::new(Mutex::new(None)),
            launch_lock: Mutex::new(()),
            artifact_dir,
        }
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
        let mut builder = BrowserConfig::builder().window_size(1280, 900);
        if let Some(exe) = &self.executable {
            builder = builder.chrome_executable(exe);
        }
        if let Some(dir) = &self.user_data_dir {
            builder = builder.user_data_dir(dir);
        }
        if self.headless {
            builder = builder.headless_mode(chromiumoxide::browser::HeadlessMode::True);
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
        *self.handler.lock().await = Some(handler);
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

    /// The frontmost page, or an error when the browser has no tabs.
    async fn current_page(&self) -> anyhow::Result<chromiumoxide::page::Page> {
        self.with_browser(|b| {
            Box::pin(async move {
                b.pages()
                    .await?
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("no active page; call navigate first"))
            })
        })
        .await
    }
}

#[async_trait]
impl BrowserTool for ChromiumBackend {
    async fn invoke(&self, call: BrowserCall) -> anyhow::Result<BrowserResponse> {
        match call.action {
            BrowserAction::Navigate { url } => {
                self.with_browser(|b| {
                    Box::pin(async move {
                        let page = b.new_page(&url).await?;
                        let _ = page.wait_for_navigation().await;
                        let final_url = page
                            .url()
                            .await
                            .unwrap_or(None)
                            .unwrap_or_else(|| url.clone());
                        let title = page.get_title().await.unwrap_or(None);
                        Ok(BrowserResponse::ok(
                            json!({ "url": final_url, "title": title }),
                        ))
                    })
                })
                .await
            }
            BrowserAction::Info => {
                self.with_browser(|b| {
                    Box::pin(async move {
                        let pages = b.pages().await?;
                        let mut out = Vec::new();
                        for page in pages {
                            if let Ok(url) = page.url().await {
                                out.push(
                                    json!({ "url": url, "title": page.get_title().await.ok() }),
                                );
                            }
                        }
                        Ok(BrowserResponse::ok(json!({ "pages": out })))
                    })
                })
                .await
            }
            BrowserAction::Close => {
                let mut guard = self.browser.lock().await;
                if let Some(browser) = guard.as_mut() {
                    let _ = browser.close().await;
                }
                *guard = None;
                self.handler.lock().await.take();
                Ok(BrowserResponse::ok(json!({ "closed": true })))
            }
            _ => Ok(BrowserResponse::err(
                "this action runs against the current page; call navigate first",
            )),
        }
    }
}

/// Actions that operate on the *current* page.
pub async fn perform_on_current_page(
    backend: &ChromiumBackend,
    profile: &BrowserProfile,
    action: BrowserAction,
) -> anyhow::Result<BrowserResponse> {
    let _ = profile;
    match action {
        BrowserAction::Dom { selector } => {
            run_js(backend, &crate::dom::inner_html_script(selector.as_deref())).await
        }
        BrowserAction::Screenshot { full_page } => screenshot(backend, full_page).await,
        BrowserAction::Evaluate { script } => run_js(backend, &script).await,
        BrowserAction::WaitFor {
            condition,
            timeout_ms,
        } => wait_for(backend, condition, timeout_ms).await,
        BrowserAction::Scroll { x, y } => {
            run_js(
                backend,
                &format!("window.scrollBy({}, {}); window.scrollY", x, y),
            )
            .await
        }
        BrowserAction::Click { selector } => {
            run_js(
                backend,
                &format!(
                    r#"(() => {{ const el = document.querySelector({sel});
                       if (!el) return false; el.click(); return true; }})()"#,
                    sel = crate::dom::js_string(&selector)
                ),
            )
            .await
        }
        BrowserAction::Type { selector, text } => {
            run_js(
                backend,
                &format!(
                    r#"(() => {{
                      const el = document.querySelector({sel});
                      if (!el) return false;
                      el.focus();
                      el.value = {val};
                      el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                      el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                      return true;
                    }})()"#,
                    sel = crate::dom::js_string(&selector),
                    val = crate::dom::js_string(&text)
                ),
            )
            .await
        }
        BrowserAction::Navigate { url } => {
            let call = BrowserCall {
                workspace_id: uuid::Uuid::nil(),
                action: BrowserAction::Navigate { url },
            };
            backend.invoke(call).await
        }
        BrowserAction::Info | BrowserAction::Close => {
            let action = if matches!(action, BrowserAction::Close) {
                BrowserAction::Close
            } else {
                BrowserAction::Info
            };
            let call = BrowserCall {
                workspace_id: uuid::Uuid::nil(),
                action,
            };
            backend.invoke(call).await
        }
    }
}

/// Evaluate JavaScript in the current page and return the result.
pub async fn run_js(backend: &ChromiumBackend, script: &str) -> anyhow::Result<BrowserResponse> {
    let page = backend.current_page().await?;
    let result = page.evaluate(script).await?;
    let value = result.value().cloned().unwrap_or(Value::Null);
    Ok(BrowserResponse::ok(value))
}

/// Capture a screenshot of the current page.
pub async fn screenshot(
    backend: &ChromiumBackend,
    full_page: bool,
) -> anyhow::Result<BrowserResponse> {
    let page = backend.current_page().await?;
    let params = chromiumoxide::page::ScreenshotParams::builder()
        .full_page(full_page)
        .build();
    let png = page.screenshot(params).await?;
    let id = uuid::Uuid::new_v4().to_string();
    tokio::fs::create_dir_all(&backend.artifact_dir).await?;
    let path = backend.artifact_dir.join(format!("{}.png", id));
    tokio::fs::write(&path, &png).await?;
    Ok(BrowserResponse {
        ok: true,
        payload: json!({ "bytes": png.len(), "full_page": full_page }),
        artifact_id: Some(id),
        error: None,
    })
}

/// Poll a condition until it holds or the timeout expires.
pub async fn wait_for(
    backend: &ChromiumBackend,
    condition: WaitCondition,
    timeout_ms: u64,
) -> anyhow::Result<BrowserResponse> {
    let script = match &condition {
        WaitCondition::Selector { selector } => crate::dom::selector_probe(selector),
        WaitCondition::Url { url } => {
            format!("window.location.href === {}", crate::dom::js_string(url))
        }
        WaitCondition::Function { script } => script.clone(),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        if let Ok(response) = run_js(backend, &script).await {
            if response.payload.as_bool() == Some(true) {
                return Ok(response);
            }
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Ok(BrowserResponse::err(format!(
        "condition not met within {} ms",
        timeout_ms
    )))
}

/// Wait for a navigation event on the current page.
pub async fn wait_for_navigation(backend: &ChromiumBackend) -> anyhow::Result<BrowserResponse> {
    let page = backend.current_page().await?;
    let _ = page.wait_for_navigation().await;
    Ok(BrowserResponse::ok(json!({
        "url": page.url().await.unwrap_or_default(),
    })))
}

/// Keep the `FutureExt` import meaningful for callers that chain onto these.
pub fn boxed<F>(fut: F) -> futures_util::future::BoxFuture<'static, anyhow::Result<()>>
where
    F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    fut.boxed()
}
