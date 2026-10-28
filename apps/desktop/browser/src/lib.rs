//! Browser Operator and backends.
//!
//! Architecture:
//!
//! ```text
//! Agent (mixd-core::runs::engine)
//!   │
//!   ▼ call_tool("browser", action=..., args=...)
//! Browser Tool (mixd-tools::registry)
//!   │
//!   ▼ trait BrowserTool
//! Browser Tool Interface (this crate)
//!   │
//!   ▼
//! Browser Operator (operator.rs)  ← Workspace Profile を保持
//!   │  navigate / click / type / scroll /
//!   │  dom / screenshot / evaluate / wait_for
//!   ▼
//! Backend: enum BrowserBackend (Chromium | SystemChromium | Disabled)
//!   │
//!   ▼
//! Managed Chromium in `<app-data>/chromium` (falls back to the system install)
//! ```
//!
//! `Agent Core` never depends on `chromiumoxide`; swapping the backend only
//! touches this crate.

pub mod artifact;
pub mod chromiumoxide_backend;
pub mod dom;
pub mod installer;
pub mod interface;
pub mod operator;
pub mod profile;

pub use artifact::{ArtifactKind, BrowserArtifact};
pub use chromiumoxide_backend::ChromiumBackend;
pub use installer::{
    artifact as chromium_artifact, manifest as chromium_manifest, platform_id as chromium_platform,
    BrowserInstallState, BrowserUpdateInfo, ChromiumArtifact, ChromiumInstaller, ChromiumManifest,
    InstallProgress, InstallStatus, InstalledChromium,
};
pub use interface::{
    browser_contract, browser_tool_spec, desktop_agent_actions, desktop_contract_actions,
    is_forbidden_ip, is_private_ip, reject_private_navigate_url,
    reject_private_navigate_url_resolved, reject_private_navigate_url_resolved_with,
    reject_private_navigate_url_with, validate_evaluate_script, BrowserAction, BrowserCall,
    BrowserResponse, BrowserTool, WaitCondition, BROWSER_CONTRACT_JSON, MAX_DOM_CHARS,
    MAX_EXTRACT_ITEMS, MAX_SCREENSHOT_BYTES, MAX_SNAPSHOT_NODES, MAX_TEXT_CHARS,
};
pub use operator::{BrowserBackend, BrowserOperator, BrowserSource};
pub use profile::{
    default_profile_root, find_bundled_chromium, find_chromium_in_root, find_downloaded_chromium,
    find_override_chromium, find_system_chromium, resolve_resource_dir, BrowserProfile,
};
