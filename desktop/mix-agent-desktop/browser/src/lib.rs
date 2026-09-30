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
//! Bundled Chromium (falls back to the system install)
//! ```
//!
//! `Agent Core` never depends on `chromiumoxide`; swapping the backend only
//! touches this crate.

pub mod artifact;
pub mod chromiumoxide_backend;
pub mod dom;
pub mod interface;
pub mod operator;
pub mod profile;

pub use artifact::{ArtifactKind, BrowserArtifact};
pub use chromiumoxide_backend::ChromiumBackend;
pub use interface::{
    browser_tool_spec, BrowserAction, BrowserCall, BrowserResponse, BrowserTool, WaitCondition,
};
pub use operator::{BrowserBackend, BrowserOperator};
pub use profile::{
    default_profile_root, find_bundled_chromium, find_system_chromium, resolve_resource_dir,
    BrowserProfile,
};
