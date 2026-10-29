//! `chromiumoxide`-backed Chromium implementation of [`BrowserTool`].
//!
//! The Agent never sees this type; the [`crate::operator::BrowserOperator`]
//! owns the lifecycle and routes calls here.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use base64::Engine as _;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::fetch::{
    ContinueRequestParams, EnableParams as FetchEnableParams, EventRequestPaused, FailRequestParams,
};
use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams as NetworkEnableParams, ErrorReason, SetBlockedUrLsParams,
};
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, CaptureScreenshotParams, EnableParams, EventScreencastFrame,
    GetNavigationHistoryParams, ScreencastFrameAckParams, StartScreencastFormat,
    StartScreencastParams, StopScreencastParams, Viewport,
};
use chromiumoxide::cdp::browser_protocol::target::TargetId;
use chromiumoxide::layout::Point;
use chromiumoxide::page::Page;
use futures_util::{FutureExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

use crate::interface::{
    BrowserAction, BrowserCall, BrowserFrame, BrowserResponse, BrowserTool, FrameSink,
    ScreencastFrame, WaitCondition, MAX_DOM_CHARS, MAX_EXTRACT_ITEMS, MAX_SCREENSHOT_BYTES,
    MAX_SNAPSHOT_NODES,
};
/// How long a Chromium launch may take before the call is failed. Launching is
/// unbounded otherwise, which would hold the launch lock forever.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);
/// A navigation that never settles must not park the whole workspace forever.
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);
/// History navigation and reload are best-effort: a page that does not emit a
/// navigation event still needs to return so the agent can inspect it.
const HISTORY_WAIT: Duration = Duration::from_secs(5);
/// Live-view frames are shown in a 380–560 px side panel. Capturing them at
/// half the CSS viewport cuts JPEG/IPC/WebView-decode cost by roughly 4x while
/// staying sharper than the panel. Click mapping is resolution-independent
/// (`framePoint` uses the ratio to the CSS viewport), so this is safe.
const LIVE_FRAME_SCALE: f64 = 0.5;
const LIVE_JPEG_QUALITY: i64 = 60;
/// CDP screencast frames arrive on every compositor paint (~15–30/s on an
/// animated page). Forwarding each one as a base64 Tauri event saturates the
/// WebView, so the sink is rate-limited independently of `every_nth_frame`.
/// 120 ms ≈ 8 fps, which is smooth enough for a 380–560 px side panel.
const LIVE_MIN_FRAME_INTERVAL: Duration = Duration::from_millis(120);
/// Per-request egress guard: re-resolve a host at most once a minute, and never
/// let a slow resolver stall a page load for longer than this.
const EGRESS_DNS_TTL: Duration = Duration::from_secs(60);
const EGRESS_DNS_TIMEOUT: Duration = Duration::from_secs(3);

/// DNS answers used by the per-request egress guard. A page can issue dozens
/// of subresource requests to the same origin, so resolving each one again
/// would add latency to every load. The cache is task-local (one guard per
/// target), so no lock is needed.
#[derive(Default)]
struct EgressDnsCache {
    entries: HashMap<String, (Instant, Vec<IpAddr>)>,
}

impl EgressDnsCache {
    async fn resolve(&mut self, host: &str, port: u16) -> Option<Vec<IpAddr>> {
        let key = format!("{host}:{port}");
        if let Some((at, addrs)) = self.entries.get(&key) {
            if at.elapsed() < EGRESS_DNS_TTL {
                return Some(addrs.clone());
            }
        }
        // Fail closed on a resolver timeout: Chromium would resolve the same
        // name anyway, and allowing the request would hand the decision back
        // to the layer this guard exists to replace.
        let resolved = timeout(EGRESS_DNS_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .ok()?
            .ok()?;
        let addrs: Vec<IpAddr> = resolved.map(|addr| addr.ip()).collect();
        if addrs.is_empty() {
            return None;
        }
        self.entries.insert(key, (Instant::now(), addrs.clone()));
        Some(addrs)
    }
}

/// Return a human-readable reason when a request must be refused.
///
/// Only `http`/`https` are gated: `data:` and `blob:` never leave the process,
/// and other schemes are already refused by `navigate`. A hostname is resolved
/// here and every answer is checked, so a name that round-robins between a
/// public and a private address cannot slip through.
async fn blocked_request(
    raw_url: &str,
    allow_local: bool,
    cache: &mut EgressDnsCache,
) -> Option<String> {
    let parsed = url::Url::parse(raw_url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let port = parsed.port_or_known_default().unwrap_or(443);

    if let Ok(ip) = bare.parse::<IpAddr>() {
        return crate::interface::is_forbidden_ip(&ip, allow_local)
            .then(|| format!("private address {ip}"));
    }

    let addrs = cache.resolve(bare, port).await?;
    addrs
        .iter()
        .find(|ip| crate::interface::is_forbidden_ip(ip, allow_local))
        .map(|ip| format!("{bare} resolves to private address {ip}"))
}

pub struct ChromiumBackend {
    executable: Option<PathBuf>,
    workspace_id: uuid::Uuid,
    /// Optional sink for CDP screencast frames. The app layer turns this into a
    /// Tauri event; tests leave it `None` and use polling instead.
    frame_sink: Option<FrameSink>,
    screencast_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Page target the current screencast is attached to. Kept so a restart
    /// stops the old target instead of leaving it unacknowledged.
    screencast_page: Mutex<Option<Page>>,
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
    /// servers). Link-local metadata stays blocked regardless. `Arc` because
    /// each page's CDP request handler reads it live.
    allow_local: Arc<AtomicBool>,
    /// Per-target request-interception tasks. Installing the guard twice for
    /// one target would make two handlers fight over each paused request, so
    /// the target id is the key.
    egress_guards: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}

impl ChromiumBackend {
    pub fn new(
        executable: Option<PathBuf>,
        user_data_dir: Option<PathBuf>,
        artifact_dir: PathBuf,
        workspace_id: uuid::Uuid,
        frame_sink: Option<FrameSink>,
    ) -> Self {
        Self {
            executable,
            workspace_id,
            frame_sink,
            screencast_task: Mutex::new(None),
            screencast_page: Mutex::new(None),
            user_data_dir,
            browser: Arc::new(Mutex::new(None)),
            handler_task: Arc::new(Mutex::new(None)),
            launch_lock: Mutex::new(()),
            active_page: Mutex::new(None),
            artifact_dir,
            allow_local: Arc::new(AtomicBool::new(false)),
            egress_guards: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_allow_local(&self, value: bool) {
        self.allow_local.store(value, Ordering::Relaxed);
    }

    pub fn allows_local(&self) -> bool {
        self.allow_local.load(Ordering::Relaxed)
    }

    /// Install CDP-level egress control on a page target.
    ///
    /// The synchronous `navigate` / `evaluate` gates only see URLs the model
    /// supplies. A page (or a legitimate page with an injected script) can
    /// still issue its own `fetch()` to `169.254.169.254`, or DNS-rebind from
    /// a public name to a private address between the pre-check and the dial.
    /// `Fetch` sees every request Chromium makes, including subresources, so
    /// the private-IP rule is enforced at the network layer instead of in a
    /// string filter.
    ///
    /// Any failure to install the guard fails the page operation: silently
    /// browsing without it would be worse than an error.
    async fn ensure_egress_guard(&self, page: &Page) -> anyhow::Result<()> {
        let key = page.target_id().inner().clone();
        // Hold the map lock across setup so two callers cannot both install a
        // handler for the same page and then race for each paused request.
        let mut guards = self.egress_guards.lock().await;
        if guards.contains_key(&key) {
            return Ok(());
        }

        // Subscribe before enabling the domain: a request that pauses between
        // `enable` and the first poll would otherwise never be answered.
        let mut events = page
            .event_listener::<EventRequestPaused>()
            .await
            .map_err(|err| anyhow::anyhow!("subscribing to page requests failed: {err}"))?;
        page.execute(NetworkEnableParams::default())
            .await
            .map_err(|err| anyhow::anyhow!("enabling network events failed: {err}"))?;
        // Defence in depth for the metadata literals: these are blocked by
        // Chromium before Fetch even pauses them, so a handler bug cannot
        // re-open the most valuable SSRF target.
        page.execute(SetBlockedUrLsParams::new(vec![
            "http://169.254.169.254/*".into(),
            "https://169.254.169.254/*".into(),
            "http://169.254.170.2/*".into(),
            "https://169.254.170.2/*".into(),
            "http://100.100.100.200/*".into(),
            "https://100.100.100.200/*".into(),
            "http://[fd00:ec2::254]/*".into(),
            "https://[fd00:ec2::254]/*".into(),
        ]))
        .await
        .map_err(|err| anyhow::anyhow!("blocking metadata URLs failed: {err}"))?;
        page.execute(FetchEnableParams::default())
            .await
            .map_err(|err| anyhow::anyhow!("enabling request interception failed: {err}"))?;

        let allow_local = Arc::clone(&self.allow_local);
        let page_for_task = page.clone();
        let task = tokio::spawn(async move {
            let mut cache = EgressDnsCache::default();
            while let Some(event) = events.next().await {
                let allow_local = allow_local.load(Ordering::Relaxed);
                let url = event.request.url.clone();
                let blocked = blocked_request(&url, allow_local, &mut cache).await;
                let outcome = match blocked {
                    Some(reason) => {
                        tracing::debug!(%url, %reason, "browser egress blocked");
                        page_for_task
                            .execute(FailRequestParams::new(
                                event.request_id.clone(),
                                ErrorReason::BlockedByClient,
                            ))
                            .await
                            .map(|_| ())
                    }
                    None => page_for_task
                        .execute(ContinueRequestParams::new(event.request_id.clone()))
                        .await
                        .map(|_| ()),
                };
                if outcome.is_err() {
                    // The page or the target is gone; stop polling this stream.
                    break;
                }
            }
        });
        guards.insert(key, task);
        Ok(())
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
            .arg("--disable-new-content-rendering-timeout")
            // Trim the parts of a full browser the agent never needs. These are
            // all launch-time features, so they cost nothing at runtime.
            .arg("--disable-background-networking")
            .arg("--disable-component-update")
            .arg("--disable-default-apps")
            .arg("--disable-sync")
            .arg("--disable-breakpad")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--mute-audio");
        if let Some(exe) = &self.executable {
            builder = builder.chrome_executable(exe);
        }
        if let Some(dir) = &self.user_data_dir {
            builder = builder.user_data_dir(dir);
        }
        // The browser is always headless. The live-view panel is the only
        // surface: a second visible Chromium window duplicated the page and
        // kept a compositor around for it. Chromium 132+ removed the old
        // headless mode, so use the new mode (which supports screenshots,
        // screencast and real input).
        builder = builder.headless_mode(chromiumoxide::browser::HeadlessMode::New);
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
            self.ensure_egress_guard(&page).await?;
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
        self.ensure_egress_guard(&page).await?;
        Ok(page)
    }

    async fn set_active(&self, page: &Page) {
        *self.active_page.lock().await = Some(page.clone());
        if let Err(err) = self.ensure_egress_guard(page).await {
            tracing::error!(%err, "installing the browser egress guard failed");
        }
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
        self.ensure_egress_guard(&page).await?;
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
    /// `clip` is also how the live view asks for a half-resolution frame;
    /// `jpeg_quality` switches the encoding: PNG for artifacts, JPEG with
    /// `optimize_for_speed` for the live-view stream.
    async fn screenshot_bytes(
        &self,
        clip: Option<Viewport>,
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
        if let Some(clip) = clip {
            builder = builder.clip(clip);
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
        let clip = if full_page {
            let page = self.current_page().await?;
            let metrics = page.layout_metrics().await?;
            Some(Viewport {
                x: 0.,
                y: 0.,
                width: metrics.css_content_size.width,
                height: metrics.css_content_size.height,
                scale: 1.,
            })
        } else {
            None
        };
        let png = match self.screenshot_bytes(clip, None).await {
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
    /// poller/fallback, where writing one file per frame would grow the disk
    /// without bound.
    ///
    /// JPEG + `optimize_for_speed` keeps the poll cheap; artifacts keep PNG.
    /// The CSS viewport dimensions travel with the image so the UI can map a
    /// click to page coordinates even when the screenshot's pixel dimensions
    /// differ (Retina / device-scale-factor).
    ///
    /// Page metadata is deliberately *not* collected here: the live view asks
    /// for `Action::Info` in the same tick, and fetching URL/title/history
    /// again would add three CDP round-trips to every 2 s poll.
    pub async fn capture_frame(&self, full_page: bool) -> anyhow::Result<BrowserFrame> {
        let page = self.current_page().await?;
        let metrics = page.layout_metrics().await?;
        let (width, height, scale) = if full_page {
            (
                metrics.css_content_size.width,
                metrics.css_content_size.height,
                1.0,
            )
        } else {
            (
                metrics.css_layout_viewport.client_width as f64,
                metrics.css_layout_viewport.client_height as f64,
                LIVE_FRAME_SCALE,
            )
        };
        let clip = Some(Viewport {
            x: 0.,
            y: 0.,
            width: width.max(1.0),
            height: height.max(1.0),
            scale,
        });
        let jpeg = self.screenshot_bytes(clip, Some(LIVE_JPEG_QUALITY)).await?;
        Ok(BrowserFrame {
            jpeg,
            width: width.max(1.0) as u32,
            height: height.max(1.0) as u32,
        })
    }

    async fn navigation_flags(&self, page: &Page) -> (bool, bool) {
        match page.execute(GetNavigationHistoryParams {}).await {
            Ok(response) => {
                let history = response.result;
                let index = history.current_index.max(0) as usize;
                (index > 0, index + 1 < history.entries.len())
            }
            // A page that denies history (e.g. a fresh about:blank) simply
            // leaves both buttons disabled; this is not an error worth failing
            // the frame over.
            Err(_) => (false, false),
        }
    }

    /// Start the CDP screencast. Frames are acknowledged and forwarded to the
    /// sink installed by the app layer (a Tauri event). Polling remains as a
    /// fallback when the sink is absent, e.g. in tests.
    pub async fn start_screencast(&self) -> anyhow::Result<()> {
        let Some(sink) = self.frame_sink.clone() else {
            return Ok(());
        };
        if let Some(previous) = self.screencast_task.lock().await.take() {
            previous.abort();
        }
        if let Some(previous_page) = self.screencast_page.lock().await.take() {
            // Aborting the listener without telling Chromium leaves the old
            // target producing frames that nobody acknowledges, which backs up
            // its compositor queue.
            let _ = previous_page.execute(StopScreencastParams {}).await;
        }
        let page = self.current_page().await?;
        let metrics = page.layout_metrics().await?;
        let width = metrics.css_layout_viewport.client_width.max(1) as u32;
        let height = metrics.css_layout_viewport.client_height.max(1) as u32;
        // Page events (including `screencastFrame`) are only delivered after
        // the domain is enabled. chromiumoxide does not enable it implicitly.
        page.execute(EnableParams::builder().build()).await?;
        // A background page produces no compositor frames. Bringing it to the
        // front makes the headless target "visible" for the screencast.
        let _ = page.bring_to_front().await;
        page.execute(
            StartScreencastParams::builder()
                .format(StartScreencastFormat::Jpeg)
                .quality(LIVE_JPEG_QUALITY)
                .max_width(1_280)
                .max_height(900)
                .every_nth_frame(2)
                .build(),
        )
        .await?;
        // Register the listener synchronously before returning. The caller
        // often navigates immediately; if subscription were left to the
        // spawned task, that first repaint could be lost.
        let mut events = page.event_listener::<EventScreencastFrame>().await?;

        *self.screencast_page.lock().await = Some(page.clone());
        let workspace_id = self.workspace_id;
        let ack_page = page.clone();
        let handle = tokio::spawn(async move {
            let mut last_emit: Option<std::time::Instant> = None;
            while let Some(frame) = events.next().await {
                // Ack first: Chromium stops sending when the queue backs up.
                if let Err(err) = ack_page
                    .execute(ScreencastFrameAckParams::new(frame.session_id))
                    .await
                {
                    tracing::debug!(%err, "acknowledging a screencast frame failed");
                }
                // Drop frames we could not render in time instead of queueing
                // them in the WebView. Acking above keeps Chromium flowing.
                let now = std::time::Instant::now();
                if last_emit.is_some_and(|at| now.duration_since(at) < LIVE_MIN_FRAME_INTERVAL) {
                    continue;
                }
                last_emit = Some(now);
                sink(ScreencastFrame {
                    workspace_id,
                    jpeg_base64: AsRef::<str>::as_ref(&frame.data).to_string(),
                    width,
                    height,
                });
            }
        });
        *self.screencast_task.lock().await = Some(handle);
        Ok(())
    }

    pub async fn stop_screencast(&self) {
        if let Some(task) = self.screencast_task.lock().await.take() {
            task.abort();
        }
        // Do not call `current_page()` here: stopping a pane that was already
        // idle-closed must not relaunch Chromium just to send StopScreencast.
        // Use the page the stream was started on: after a tab switch the
        // active page is already the new one, and stopping there would leave
        // the old target streaming.
        if let Some(page) = self.screencast_page.lock().await.take() {
            let _ = page.execute(StopScreencastParams {}).await;
        }
    }

    /// Restart the stream after the active page target changed (tab switch,
    /// new tab, closing the active tab). The UI only restarts for its own
    /// actions; without this an agent-driven tab change keeps showing the old
    /// page until the pane is reopened.
    async fn restart_screencast_if_active(&self) {
        if self.screencast_task.lock().await.is_none() {
            return;
        }
        if let Err(err) = self.start_screencast().await {
            tracing::debug!(%err, "restarting the screencast after a tab change failed");
        }
    }

    /// Stop loading the current tab (the toolbar's stop button).
    pub async fn stop_loading(&self) -> anyhow::Result<()> {
        let page = self.current_page().await?;
        page.execute(chromiumoxide::cdp::browser_protocol::page::StopLoadingParams {})
            .await?;
        Ok(())
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
        let tab_changed = matches!(
            &call.action,
            BrowserAction::SwitchTab { .. }
                | BrowserAction::NewTab { .. }
                | BrowserAction::CloseTab { .. }
        );
        let result = match call.action {
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
                let (can_go_back, can_go_forward) = self.navigation_flags(&page).await;
                Ok(BrowserResponse::ok(json!({
                    "pages": pages,
                    "active": {
                        "id": active,
                        "url": page.url().await.unwrap_or(None),
                        "title": page.get_title().await.unwrap_or(None),
                    },
                    "can_go_back": can_go_back,
                    "can_go_forward": can_go_forward,
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
                if let Some(task) = self.egress_guards.lock().await.remove(&closed_id) {
                    task.abort();
                }
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
                self.stop_screencast().await;
                let mut guard = self.browser.lock().await;
                if let Some(browser) = guard.as_mut() {
                    let _ = browser.close().await;
                }
                *guard = None;
                if let Some(task) = self.handler_task.lock().await.take() {
                    task.abort();
                }
                let mut guards = self.egress_guards.lock().await;
                for (_, task) in guards.drain() {
                    task.abort();
                }
                drop(guards);
                *self.active_page.lock().await = None;
                Ok(BrowserResponse::ok(json!({ "closed": true })))
            }
        };
        // CDP screencasts are bound to one page target. Follow an
        // agent-driven tab change here; the UI only restarts the stream for
        // its own toolbar actions.
        if tab_changed && result.is_ok() {
            self.restart_screencast_if_active().await;
        }
        result
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

    #[tokio::test]
    async fn request_egress_guard_blocks_metadata_and_private_literals() {
        let mut cache = EgressDnsCache::default();
        // Link-local metadata is never reachable, even with the local opt-in.
        for allow_local in [false, true] {
            assert!(blocked_request(
                "http://169.254.169.254/latest/meta-data/",
                allow_local,
                &mut cache
            )
            .await
            .is_some());
        }
        // Loopback / RFC1918 follow the workspace opt-in.
        assert!(blocked_request("http://127.0.0.1:3000/", false, &mut cache)
            .await
            .is_some());
        assert!(blocked_request("http://127.0.0.1:3000/", true, &mut cache)
            .await
            .is_none());
        assert!(blocked_request("http://192.168.1.10/", false, &mut cache)
            .await
            .is_some());
        assert!(blocked_request("http://192.168.1.10/", true, &mut cache)
            .await
            .is_none());
        // Public literals and non-network schemes pass through.
        assert!(blocked_request("https://93.184.216.34/", false, &mut cache)
            .await
            .is_none());
        assert!(blocked_request("data:text/plain,hi", false, &mut cache)
            .await
            .is_none());
    }

    #[test]
    fn text_truncation_is_character_based() {
        assert_eq!(truncate_chars("abc", 5), "abc");
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
        // Multi-byte input must not be cut mid-codepoint.
        assert_eq!(truncate_chars("あいうえお", 2), "あい…");
    }
}
