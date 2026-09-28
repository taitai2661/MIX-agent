//! `#[tauri::command]` surface.
//!
//! This module is the **only** trust boundary between the WebView and the Rust
//! core. Every operation the UI can trigger is implemented here, and the WebView
//! is denied direct access to `fs`, `shell`, and `http` plugins.

pub mod agents;
pub mod approvals;
pub mod backups;
pub mod browser;
pub mod conversations;
pub mod diagnostics;
pub mod fs;
pub mod health;
pub mod knowledge;
pub mod mcp;
pub mod memories;
pub mod models;
pub mod projects;
pub mod providers;
pub mod runs;
pub mod schedules;
pub mod settings;
pub mod skills;
pub mod tools;
pub mod workspace;
pub mod zip_util;
