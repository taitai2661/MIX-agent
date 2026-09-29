//! Error formatting helpers for the Tauri command boundary.
//!
//! Every `#[tauri::command]` returns its failures as a `String` that the
//! WebView displays verbatim. Using `Display` (`to_string()`) on an
//! `anyhow::Error` only prints the outermost message and throws away the
//! context chain that was painstakingly attached while the error bubbled up,
//! so the user ends up seeing things like "database error" with no hint about
//! which statement or which record failed.
//!
//! [`cmd_err`] formats with `{:#}`, which for `anyhow::Error` renders the full
//! chain (`outer: middle: root`) and for every other `Display` type is
//! identical to `{}`. That makes it a safe, mechanical replacement for
//! `.map_err(|e| e.to_string())` at the boundary.

/// Format an error for transport across the Tauri IPC boundary.
///
/// `anyhow::Error` keeps its whole context chain; other error types render as
/// plain `Display`.
pub fn cmd_err<E: std::fmt::Display>(err: E) -> String {
    format!("{err:#}")
}
