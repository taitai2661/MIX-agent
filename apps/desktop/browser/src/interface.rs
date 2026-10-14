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
    Click {
        selector: String,
    },
    Type {
        selector: String,
        text: String,
    },
    Scroll {
        x: i32,
        y: i32,
    },
    Dom {
        selector: Option<String>,
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
    Close,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitCondition {
    Selector { selector: String },
    Url { url: String },
    Function { script: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResponse {
    pub ok: bool,
    pub payload: serde_json::Value,
    pub artifact_id: Option<String>,
    pub error: Option<String>,
}

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
/// TODO(CDP): a substring gate cannot stop `eval("fe"+"tch")`, a redirect
/// chain, or a same-origin page that itself fetches metadata. The durable fix
/// is CDP-level egress control (`Network.setBlockedURLs` / request
/// interception on the browser target, plus blocking private-IP literals after
/// DNS in `Fetch.requestPaused`), applied to every page the operator owns —
/// not just to model-supplied strings. Until that lands, treat this as
/// defence-in-depth, not a boundary.
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

/// Refuse `navigate` targets that resolve to a private literal IP.
///
/// This is the browser-side equivalent of the tools crate's egress
/// private-IP deny, kept local (rather than depending on `mixd-tools`) so the
/// browser crate stays backend-swappable. Only `http`/`https` URLs are gated;
/// other schemes pass through.
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
    let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid navigate url: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Ok(()),
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
        if is_private_ip(&ip) {
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
    reject_private_navigate_url(url)?;
    let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid navigate url: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
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
                if is_private_ip(&addr.ip()) {
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

fn is_private_ip(ip: &std::net::IpAddr) -> bool {
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
        (0xc0a8_0000, 16), // 192.168.0.0/16
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
    serde_json::json!({
        "name": "browser",
        "description": "Drive a real Chromium instance: navigate, click, type, scroll, read the DOM, screenshot, evaluate JavaScript, and wait for conditions. Each workspace keeps its own persistent profile so logins survive between runs.",
        "parameters": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "click", "type", "scroll", "dom", "screenshot", "evaluate", "wait_for", "info", "close"],
                    "description": "Operation to perform."
                },
                "url": { "type": "string", "description": "Target URL for navigate / wait_for(url)." },
                "selector": { "type": "string", "description": "CSS selector for click / type / dom / wait_for(selector)." },
                "text": { "type": "string", "description": "Text to type." },
                "x": { "type": "integer", "description": "Scroll delta X." },
                "y": { "type": "integer", "description": "Scroll delta Y." },
                "script": { "type": "string", "description": "JavaScript for evaluate / wait_for(function)." },
                "full_page": { "type": "boolean", "default": false, "description": "Capture the full scrollable page." },
                "timeout_ms": { "type": "integer", "default": 10000 }
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
