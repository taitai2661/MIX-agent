//! Browser Tool contract shared by the Desktop agent and the Tool Registry.
//!
//! This crate deliberately contains no browser engine. The Desktop build
//! implements [`BrowserHost`] with an in-window Tauri child WebView; Docker
//! keeps its Playwright `browser_*` tools. Keeping the action vocabulary and
//! limits here means the two editions cannot drift without the parity check
//! noticing.
//!
//! ```text
//! Agent (mixd-core::runs::engine)
//!   │
//!   ▼ call_tool("browser", action=..., args=...)
//! Browser Tool (mixd-tools::registry)
//!   │
//!   ▼ trait BrowserHost
//! Browser Tool Contract (this crate)
//!   │
//!   ▼
//! Desktop: Tauri child WebView + JS bridge (app/src/webview_browser)
//! Docker:  Playwright browser_* tools
//! ```

pub mod artifact;
pub mod dom;
pub mod interface;

pub use artifact::{ArtifactKind, BrowserArtifact};
pub use interface::{
    browser_contract, browser_tool_spec, desktop_agent_actions, desktop_contract_actions,
    is_forbidden_ip, is_private_ip, reject_private_navigate_url,
    reject_private_navigate_url_resolved, reject_private_navigate_url_resolved_with,
    reject_private_navigate_url_with, validate_evaluate_script, BrowserAction, BrowserCall,
    BrowserHost, BrowserResponse, WaitCondition, BROWSER_CONTRACT_JSON, MAX_DOM_CHARS,
    MAX_EVALUATE_SCRIPT_BYTES, MAX_EXTRACT_ITEMS, MAX_SCREENSHOT_BYTES, MAX_SNAPSHOT_NODES,
    MAX_TEXT_CHARS,
};
