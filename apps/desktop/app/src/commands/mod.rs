//! `#[tauri::command]` surface.
//!
//! This module is the **only** trust boundary between the WebView and the Rust
//! core. Every operation the UI can trigger is implemented here, and the WebView
//! is denied direct access to `fs`, `shell`, and `http` plugins.

pub mod access;
pub mod agents;
pub mod approvals;
pub mod auto;
pub mod backups;
pub mod browser;
pub mod conversations;
pub mod diagnostics;
pub mod discovery;
pub mod fs;
pub mod health;
pub mod knowledge;
pub mod local_runtime;
pub mod mcp;
pub mod memories;
pub mod models;
pub mod picker;
pub mod projects;
pub mod providers;
pub mod runs;
pub mod schedules;
pub mod settings;
pub mod skills;
pub mod tools;
pub mod update;
pub mod workspace;
pub mod zip_util;

/// Re-exported so command modules can write `.map_err(crate::error::cmd_err)`
/// (or the shorter `super::cmd_err`) when handing an `anyhow` chain to the
/// WebView without losing the context.
pub use crate::error::cmd_err;
