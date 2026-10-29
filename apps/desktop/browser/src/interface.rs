//! The Browser Tool contract surfaced to the Tool Registry.
//!
//! Concrete backends live behind [`crate::operator::BrowserOperator`], so the
//! Agent never depends on `chromiumoxide` directly.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCall {
    pub workspace_id: uuid::Uuid,
    pub action: BrowserAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserAction {
    Navigate {
        url: String,
    },
    /// Click an element. `selector` stays a plain string so older callers keep
    /// working; `ref` is the stable handle produced by `snapshot`.
    Click {
        selector: String,
        #[serde(default, rename = "ref")]
        reference: Option<String>,
    },
    /// Click viewport coordinates (manual takeover).
    ClickAt {
        x: i32,
        y: i32,
    },
    Type {
        selector: String,
        text: String,
        #[serde(default, rename = "ref")]
        reference: Option<String>,
        #[serde(default)]
        clear: bool,
    },
    /// Type into the currently focused element (manual takeover).
    TypeActive {
        text: String,
    },
    Press {
        key: String,
    },
    Hover {
        selector: String,
        #[serde(default, rename = "ref")]
        reference: Option<String>,
    },
    Select {
        selector: String,
        value: String,
        #[serde(default, rename = "ref")]
        reference: Option<String>,
    },
    Check {
        selector: String,
        checked: bool,
        #[serde(default, rename = "ref")]
        reference: Option<String>,
    },
    Scroll {
        x: i32,
        y: i32,
    },
    Dom {
        selector: Option<String>,
    },
    /// Structured text/link extraction for evidence gathering.
    Extract {
        selector: String,
        #[serde(default = "default_extract_limit")]
        limit: usize,
    },
    /// Accessibility-oriented element tree with refs for subsequent actions.
    Snapshot {
        #[serde(default = "default_snapshot_nodes")]
        max_nodes: usize,
    },
    Screenshot {
        full_page: bool,
    },
    Evaluate {
        script: String,
    },
    WaitFor {
        condition: WaitCondition,
        timeout_ms: u64,
    },
    /// Read the current URL and title.
    Info,
    Reload,
    Back,
    Forward,
    Tabs,
    NewTab {
        #[serde(default)]
        url: Option<String>,
    },
    SwitchTab {
        tab_id: String,
    },
    CloseTab {
        #[serde(default)]
        tab_id: Option<String>,
    },
    Close,
}

fn default_extract_limit() -> usize {
    crate::interface::MAX_EXTRACT_ITEMS
}

fn default_snapshot_nodes() -> usize {
    crate::interface::MAX_SNAPSHOT_NODES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitCondition {
    Selector {
        selector: String,
    },
    Url {
        url: String,
    },
    Text {
        text: String,
    },
    /// Wait until `document.readyState === "complete"`.
    Load,
    Function {
        script: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResponse {
    pub ok: bool,
    pub payload: serde_json::Value,
    pub artifact_id: Option<String>,
    pub error: Option<String>,
}

/// One non-persisted live-view frame plus the page metadata the side panel
/// needs to draw its toolbar. The JPEG is encoded at half the CSS viewport;
/// `width`/`height` are the CSS viewport, not the image pixels.
#[derive(Debug, Clone)]
pub struct BrowserFrame {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A screencast frame pushed from the CDP handler. `jpeg_base64` is already
/// base64-encoded by Chromium, so the app layer can put it straight into a
/// `data:` URL without a second encode.
#[derive(Debug, Clone)]
pub struct ScreencastFrame {
    pub workspace_id: uuid::Uuid,
    pub jpeg_base64: String,
    pub width: u32,
    pub height: u32,
}

pub type FrameSink = std::sync::Arc<dyn Fn(ScreencastFrame) + Send + Sync>;

impl BrowserResponse {
    pub fn ok(payload: serde_json::Value) -> Self {
        Self {
            ok: true,
            payload,
            artifact_id: None,
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            ok: false,
            payload: serde_json::Value::Null,
            artifact_id: None,
            error: Some(message),
        }
    }
}

#[async_trait]
pub trait BrowserTool: Send + Sync {
    async fn invoke(&self, call: BrowserCall) -> anyhow::Result<BrowserResponse>;
}

/// Hard ceiling on an `evaluate` / `wait_for(function)` script.
///
/// An unbounded script is an unbounded prompt injection into the page: the
/// model could paste megabytes of JS and blow up the CDP message or the page
/// itself. 16 KB is generous for any legitimate probe.
pub const MAX_EVALUATE_SCRIPT_BYTES: usize = 16 * 1024;

/// Caps shared with the canonical contract. A page can produce megabytes of
/// `innerHTML`; returning that unchanged burns the model's context and the
/// user's budget, so every text-producing action truncates.
pub const MAX_DOM_CHARS: usize = 200_000;
pub const MAX_TEXT_CHARS: usize = 30_000;
pub const MAX_SNAPSHOT_NODES: usize = 300;
pub const MAX_SCREENSHOT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EXTRACT_ITEMS: usize = 100;

/// The cross-edition action contract, embedded at compile time. Keeping it as
/// data means Desktop and Docker cannot drift on action names without a test
/// (and a CI parity script) noticing.
pub const BROWSER_CONTRACT_JSON: &str =
    include_str!("../../../../packages/tool_contracts/browser.json");

pub fn browser_contract() -> serde_json::Value {
    serde_json::from_str(BROWSER_CONTRACT_JSON)
        .expect("packages/tool_contracts/browser.json must be valid JSON")
}

/// Actions the Desktop surface advertises, in contract order.
pub fn desktop_contract_actions() -> Vec<String> {
    browser_contract()["surfaces"]["desktop"]["expose"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Actions the model may choose. `click_at` and `type_active` exist for the
/// manual live-view surface; advertising raw viewport coordinates and typing
/// into whatever happens to have focus invites blind clicks. The contract
/// marks them `manual_only`, and the Agent tool spec filters them out while the
/// human-facing commands keep accepting them.
pub fn desktop_agent_actions() -> Vec<String> {
    let contract = browser_contract();
    let manual: std::collections::HashSet<&str> = contract["surfaces"]["desktop"]["manual_only"]
        .as_array()
        .map(|items| items.iter().filter_map(|item| item.as_str()).collect())
        .unwrap_or_default();
    desktop_contract_actions()
        .into_iter()
        .filter(|action| !manual.contains(action.as_str()))
        .collect()
}

/// Reject empty scripts, scripts over [`MAX_EVALUATE_SCRIPT_BYTES`], and
/// scripts that reach for the network.
///
/// `evaluate` runs with the page's ambient authority (cookies, intranet
/// access), so a model-supplied `fetch("http://169.254.169.254/")` inside an
/// otherwise innocent probe is the same SSRF as a `navigate` there — only
/// laundered through JS. The substring gate below is deliberately narrow
/// (exact `fetch(`, `XMLHttpRequest`, `WebSocket(`, `EventSource(` tokens) to
/// avoid breaking legitimate DOM reads while closing the obvious hole.
///
/// A substring gate cannot stop `eval("fe"+"tch")` or a redirect chain, so it
/// is only the first layer. The durable check lives in
/// `ChromiumBackend::ensure_egress_guard`: every page target gets
/// `Fetch` request interception, which resolves the host and refuses private /
/// link-local answers before Chromium dials. This function stays as a cheap
/// early rejection that produces a useful tool error instead of a failed page
/// load.
pub fn validate_evaluate_script(script: &str) -> anyhow::Result<()> {
    if script.trim().is_empty() {
        anyhow::bail!("evaluate script must not be empty");
    }
    if script.len() > MAX_EVALUATE_SCRIPT_BYTES {
        anyhow::bail!(
            "evaluate script is {} bytes, over the {} byte limit",
            script.len(),
            MAX_EVALUATE_SCRIPT_BYTES
        );
    }
    for token in ["fetch(", "XMLHttpRequest", "WebSocket(", "EventSource("] {
        if script.contains(token) {
            anyhow::bail!(
                "evaluate script reaches the network via `{token}`; use the web_fetch tool instead"
            );
        }
    }
    Ok(())
}

/// Refuse `navigate` targets that are not a plain web URL or that resolve to a
/// private literal IP.
///
/// This is the browser-side equivalent of the tools crate's egress
/// private-IP deny, kept local (rather than depending on `mixd-tools`) so the
/// browser crate stays backend-swappable. Only `http`/`https` are accepted;
/// `about:blank` is the one non-network destination. Everything else —
/// `file:`, `data:`, `javascript:`, `chrome:` — is refused. Letting those
/// through would let a model read local files or run a script in a page that
/// never passed [`validate_evaluate_script`].
///
/// Two layers: the synchronous literal check below rejects a private IP
/// written directly in the URL (the cloud-metadata / loopback SSRF shape)
/// before Chromium ever dials it. Hostnames are *not* resolved here — use
/// [`reject_private_navigate_url_resolved`] on the async path (the Chromium
/// backend's `navigate` and `wait_for(url)` both call it) so a hostname that
/// *resolves* to a private address is refused too. A TOCTOU remains (DNS can
/// change between the check and the dial); the durable fix is CDP request
/// interception — see the TODO on [`validate_evaluate_script`].
pub fn reject_private_navigate_url(url: &str) -> anyhow::Result<()> {
    reject_private_navigate_url_with(url, false)
}

/// As [`reject_private_navigate_url`], but with an explicit local-network
/// opt-in. Even in opt-in mode the cloud metadata / link-local range stays
/// blocked: "let the agent open my Vite server" must not become "let the agent
/// read 169.254.169.254".
pub fn reject_private_navigate_url_with(url: &str, allow_private: bool) -> anyhow::Result<()> {
    let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid navigate url: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        // `about:blank` is the initial document and a safe way to clear the
        // tab. Every other non-web scheme is refused rather than passed to
        // Chromium.
        "about" if is_about_blank(&parsed) => return Ok(()),
        other => anyhow::bail!(
            "refusing to navigate to unsupported scheme `{other}`; only http, https and about:blank are allowed"
        ),
    }
    let Some(host) = parsed.host_str() else {
        anyhow::bail!("navigate url is missing a host");
    };
    // `host_str` keeps IPv6 brackets; strip them before parsing as an IP.
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
        .to_string();
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        if is_forbidden_ip(&ip, allow_private) {
            anyhow::bail!("refusing to navigate to private IP {ip}");
        }
    }
    Ok(())
}

/// [`reject_private_navigate_url`] plus DNS: resolve the hostname and refuse
/// when *any* resolved address is private.
///
/// Async because resolution is I/O; callers on the sync path (tool-argument
/// parsing) keep using the literal check, and the backend re-checks with this
/// before dialling.
pub async fn reject_private_navigate_url_resolved(url: &str) -> anyhow::Result<()> {
    reject_private_navigate_url_resolved_with(url, false).await
}

pub async fn reject_private_navigate_url_resolved_with(
    url: &str,
    allow_private: bool,
) -> anyhow::Result<()> {
    reject_private_navigate_url_with(url, allow_private)?;
    let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid navigate url: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        "about" if is_about_blank(&parsed) => return Ok(()),
        _ => return Ok(()),
    }
    let Some(host) = parsed.host_str() else {
        return Ok(());
    };
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
        .to_string();
    // A literal IP already passed the check above; nothing left to resolve.
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    // `lookup_host` resolves the name; every address is gated, not just the
    // first — a hostname round-robining between public and private answers
    // must still be refused.
    match tokio::net::lookup_host((bare, port)).await {
        Ok(addrs) => {
            for addr in addrs {
                if is_forbidden_ip(&addr.ip(), allow_private) {
                    anyhow::bail!(
                        "refusing to navigate to {url}: resolves to private IP {}",
                        addr.ip()
                    );
                }
            }
            Ok(())
        }
        Err(err) => {
            // Fail closed: an unresolvable navigation target is refused rather
            // than handed to Chromium to resolve on its own (which would skip
            // the gate entirely).
            anyhow::bail!("refusing to navigate to {url}: DNS resolution failed: {err:#}");
        }
    }
}

/// Whether an address must be refused. `allow_private` relaxes loopback,
/// RFC1918 and IPv6 unique-local, but never link-local/metadata, multicast or
/// the unspecified address.
///
/// Shared with the Tool Registry's egress policy (`mixd_tools::egress`) so the
/// browser and the HTTP tools cannot drift on what "private" means.
pub fn is_forbidden_ip(ip: &std::net::IpAddr, allow_private: bool) -> bool {
    if !allow_private {
        return is_private_ip(ip);
    }
    match ip {
        std::net::IpAddr::V4(v4) => is_forbidden_v4_with_local(*v4),
        std::net::IpAddr::V6(v6) => {
            // An IPv4-mapped address (`::ffff:169.254.169.254`) reaches the
            // IPv4 service behind it. Without this mapping the local opt-in
            // would re-open the metadata endpoint the non-opt-in path blocks.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_forbidden_v4_with_local(v4);
            }
            v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || (v6.segments()[0] & 0xff00) == 0xff00 // ff00::/8 multicast
                || v6.segments()[0] == 0x0100 // 100::/64 discard-only
        }
    }
}

fn is_forbidden_v4_with_local(v4: std::net::Ipv4Addr) -> bool {
    let raw = u32::from(v4);
    // 0.0.0.0/8, 169.254.0.0/16, 224.0.0.0/4, 240.0.0.0/4. Everything else
    // (loopback, RFC1918, CGNAT, documentation/benchmark ranges) is reachable
    // once the user opts in; the ranges here stay blocked because they are
    // metadata/unspecified or cannot be a real host.
    (raw & 0xff00_0000) == 0
        || (raw & 0xffff_0000) == 0xa9fe_0000
        || (raw & 0xf000_0000) == 0xe000_0000
        || (raw & 0xf000_0000) == 0xf000_0000
}

/// True when the parsed URL is exactly `about:blank`.
fn is_about_blank(url: &url::Url) -> bool {
    url.as_str() == "about:blank"
}

pub fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_private_v4(*v4),
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4() {
                return is_private_v4(v4);
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
                || (v6.segments()[0] & 0xff00) == 0xff00 // ff00::/8 multicast
                || v6.segments()[0] == 0x0100 // 100::/64 discard-only
        }
    }
}

/// Literal-IP ranges the browser must never navigate to: loopback, RFC1918,
/// link-local (the cloud metadata service lives at `169.254.169.254`), CGNAT,
/// and multicast/reserved space.
fn is_private_v4(ip: std::net::Ipv4Addr) -> bool {
    const BLOCKS: &[(u32, u8)] = &[
        (0x0000_0000, 8),  // 0.0.0.0/8
        (0x0a00_0000, 8),  // 10.0.0.0/8
        (0x6440_0000, 10), // 100.64.0.0/10 CGNAT
        (0x7f00_0000, 8),  // 127.0.0.0/8 loopback
        (0xa9fe_0000, 16), // 169.254.0.0/16 link-local / metadata
        (0xac10_0000, 12), // 172.16.0.0/12
        (0xc000_0000, 24), // 192.0.0.0/24 IETF protocol assignments
        (0xc000_0200, 24), // 192.0.2.0/24 TEST-NET-1
        (0xc0a8_0000, 16), // 192.168.0.0/16
        (0xc612_0000, 15), // 198.18.0.0/15 benchmarking
        (0xc633_6400, 24), // 198.51.100.0/24 TEST-NET-2
        (0xcb00_7100, 24), // 203.0.113.0/24 TEST-NET-3
        (0xe000_0000, 4),  // 224.0.0.0/4 multicast
        (0xf000_0000, 4),  // 240.0.0.0/4 reserved
    ];
    let raw = u32::from(ip);
    BLOCKS.iter().any(|(net, prefix)| {
        // A `/0` prefix would shift by the full 32 bits, which panics in
        // debug (and wraps in release): guard it the way `egress.rs` does.
        if *prefix == 0 {
            return true;
        }
        let mask = u32::MAX.checked_shl((32 - prefix) as u32).unwrap_or(0);
        (raw & mask) == (net & mask)
    })
}

/// JSON schema for the single `browser` Tool the Agent sees.
pub fn browser_tool_spec() -> serde_json::Value {
    let actions = desktop_agent_actions();
    serde_json::json!({
        "name": "browser",
        "description": "Drive a real Chromium instance: navigate, inspect an accessibility snapshot, click/type with real keyboard and mouse events, hover, select, check, scroll, read capped DOM/structured content, screenshot, evaluate JavaScript, wait for conditions, and manage tabs. Each workspace keeps its own persistent profile so logins survive between runs.",
        "parameters": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": actions,
                    "description": "Operation to perform. Prefer `snapshot` then act on a returned `ref` when CSS selectors are fragile."
                },
                "url": { "type": "string", "description": "Target URL for navigate / new_tab / wait_for(url)." },
                "selector": { "type": "string", "description": "CSS selector for click / type / hover / select / check / dom / extract / wait_for(selector)." },
                "ref": { "type": "string", "description": "Stable element ref returned by `snapshot` (e.g. r12). Preferred over selector." },
                "text": { "type": "string", "description": "Text to type, or text to wait for." },
                "key": { "type": "string", "description": "Keyboard key for `press` (Enter, Tab, Escape, ArrowDown, ...)." },
                "value": { "type": "string", "description": "Option value for `select`." },
                "checked": { "type": "boolean", "description": "Desired state for `check`." },
                "clear": { "type": "boolean", "default": false, "description": "Clear the field before typing." },
                "x": { "type": "integer", "description": "Scroll delta X / click_at coordinate." },
                "y": { "type": "integer", "description": "Scroll delta Y / click_at coordinate." },
                "script": { "type": "string", "description": "JavaScript for evaluate / wait_for(function)." },
                "condition": { "type": "string", "enum": ["selector", "url", "text", "load", "function"], "description": "Condition kind for wait_for." },
                "full_page": { "type": "boolean", "default": false, "description": "Capture the full scrollable page." },
                "max_nodes": { "type": "integer", "minimum": 1, "maximum": 300, "default": 300, "description": "Snapshot node cap." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20, "description": "Extract item cap." },
                "tab_id": { "type": "string", "description": "Target id for switch_tab / close_tab (from `tabs`)." },
                "timeout_ms": { "type": "integer", "default": 10000, "description": "Maximum wait in milliseconds (capped at 60000)." }
            },
            "required": ["action"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_literal_ips_are_rejected_for_navigate() {
        for url in [
            "http://127.0.0.1/",
            "http://10.0.0.1/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
        ] {
            assert!(
                reject_private_navigate_url(url).is_err(),
                "{url} must be blocked"
            );
        }
    }

    #[test]
    fn local_opt_in_allows_loopback_but_not_metadata() {
        for url in [
            "http://127.0.0.1:5173/",
            "http://localhost:3000/",
            "http://[::1]:5173/",
        ] {
            assert!(reject_private_navigate_url_with(url, true).is_ok());
        }
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://[fe80::1]/",
            // The IPv4-mapped spelling must not re-open the metadata service
            // once the local opt-in is on.
            "http://[::ffff:169.254.169.254]/",
        ] {
            assert!(reject_private_navigate_url_with(url, true).is_err());
        }
        assert!(reject_private_navigate_url_with("http://127.0.0.1/", false).is_err());
    }

    #[test]
    fn non_web_schemes_are_refused() {
        for url in [
            "file:///etc/passwd",
            "data:text/html,<script>fetch('https://example.com')</script>",
            "javascript:alert(1)",
            "chrome://settings",
        ] {
            assert!(
                reject_private_navigate_url(url).is_err(),
                "{url} must not reach Chromium"
            );
        }
        assert!(reject_private_navigate_url("about:blank").is_ok());
    }

    #[test]
    fn the_model_spec_hides_manual_only_actions() {
        let spec = browser_tool_spec();
        let actions = spec["parameters"]["properties"]["action"]["enum"]
            .as_array()
            .expect("enum")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert!(!actions.contains(&"click_at"));
        assert!(!actions.contains(&"type_active"));
        assert!(actions.contains(&"click"));
        assert!(actions.contains(&"snapshot"));
    }

    #[test]
    fn public_urls_pass_the_navigate_gate() {
        for url in [
            "https://example.com/",
            "https://93.184.216.34/",
            "about:blank",
        ] {
            assert!(
                reject_private_navigate_url(url).is_ok(),
                "{url} must be allowed"
            );
        }
    }

    #[test]
    fn evaluate_scripts_are_bounded_and_non_empty() {
        assert!(validate_evaluate_script("").is_err());
        assert!(validate_evaluate_script("   \n ").is_err());
        assert!(validate_evaluate_script("document.title").is_ok());
        let big = "x".repeat(MAX_EVALUATE_SCRIPT_BYTES + 1);
        assert!(validate_evaluate_script(&big).is_err());
    }

    #[test]
    fn evaluate_scripts_that_reach_the_network_are_rejected() {
        // `evaluate` runs with the page's authority, so a model-supplied
        // fetch is SSRF laundered through JS — the same shape `navigate`
        // gating exists for. Kept in step with `validate_evaluate_script`.
        for script in [
            "fetch('http://169.254.169.254/latest/meta-data/')",
            "new XMLHttpRequest()",
            "new WebSocket('ws://example.com/')",
            "new EventSource('/events')",
        ] {
            assert!(
                validate_evaluate_script(script).is_err(),
                "{script} must be blocked"
            );
        }
        assert!(validate_evaluate_script("document.querySelector('#a').textContent").is_ok());
    }
}
