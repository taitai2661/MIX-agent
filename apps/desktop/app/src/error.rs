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

/// Error code carried across the Tauri IPC boundary.
///
/// Keeps the wire format a plain `String` (WebView displays it verbatim) but
/// prefixes a machine-readable `CODE: message` so the UI can branch on
/// `DATABASE_UNAVAILABLE` vs `PROVIDER_AUTH` instead of substring-matching
/// Japanese/English prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdErrorCode {
    /// Catch-all for `anyhow` chains without a specific mapping.
    Internal,
    Database,
    ProviderAuth,
    ProviderRateLimit,
    Network,
    Filesystem,
    PermissionDenied,
    NotFound,
}

impl CmdErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Internal => "INTERNAL",
            Self::Database => "DATABASE",
            Self::ProviderAuth => "PROVIDER_AUTH",
            Self::ProviderRateLimit => "PROVIDER_RATE_LIMIT",
            Self::Network => "NETWORK",
            Self::Filesystem => "FILESYSTEM",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::NotFound => "NOT_FOUND",
        }
    }
}

/// Format an error for transport across the Tauri IPC boundary.
///
/// `anyhow::Error` keeps its whole context chain; other error types render as
/// plain `Display`.
pub fn cmd_err<E: std::fmt::Display>(err: E) -> String {
    format!("{err:#}")
}

/// Same as [`cmd_err`] but with a stable `CODE: ` prefix for programmatic
/// handling on the TypeScript side (`parseCmdError`).
pub fn cmd_err_with_code<E: std::fmt::Display>(code: CmdErrorCode, err: E) -> String {
    format!("{}: {err:#}", code.as_str())
}

/// Split a `CODE: message` payload back into its parts. Returns
/// `(CmdErrorCode::Internal, whole)` when no known prefix is present, so old
/// plain-string errors keep working.
pub fn parse_cmd_error(payload: &str) -> (CmdErrorCode, &str) {
    let (code, rest) = match payload.split_once(": ") {
        Some(pair) => pair,
        None => return (CmdErrorCode::Internal, payload),
    };
    let code = match code {
        "DATABASE" => CmdErrorCode::Database,
        "PROVIDER_AUTH" => CmdErrorCode::ProviderAuth,
        "PROVIDER_RATE_LIMIT" => CmdErrorCode::ProviderRateLimit,
        "NETWORK" => CmdErrorCode::Network,
        "FILESYSTEM" => CmdErrorCode::Filesystem,
        "PERMISSION_DENIED" => CmdErrorCode::PermissionDenied,
        "NOT_FOUND" => CmdErrorCode::NotFound,
        "INTERNAL" => CmdErrorCode::Internal,
        _ => return (CmdErrorCode::Internal, payload),
    };
    (code, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_errors_default_to_internal() {
        let (code, msg) = parse_cmd_error("database error");
        assert_eq!(code, CmdErrorCode::Internal);
        assert_eq!(msg, "database error");
    }

    #[test]
    fn round_trip_keeps_chain() {
        let err = anyhow::anyhow!("open failed").context("read settings");
        let payload = cmd_err_with_code(CmdErrorCode::Filesystem, err);
        let (code, msg) = parse_cmd_error(&payload);
        assert_eq!(code, CmdErrorCode::Filesystem);
        assert!(msg.contains("read settings"));
        assert!(msg.contains("open failed"));
    }
}
