//! Machine-readable errors for local-runtime operations.
//!
//! The UI translates Japanese strings, so a backend error has to survive the
//! `String` boundary intact. A code plus a JSON argument array does that without
//! leaking a struct through Tauri's error type:
//!
//! ```text
//! mix-local-error:spawn_failed|["/usr/local/bin/ollama","No such file"]
//! ```
//!
//! `ui/src/lib/errors.ts` recognises the prefix and renders the localized
//! template; a build that predates a new code falls back to the raw text.

use serde_json::Value;

pub const PREFIX: &str = "mix-local-error:";

/// Build an error that the UI can translate.
///
/// `args` are positional and interpolated into the template for `code` in
/// `ui/src/lib/localErrors.ts`. Keep the order stable.
pub fn error(code: &str, args: &[String]) -> anyhow::Error {
    let encoded = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
    anyhow::anyhow!("{PREFIX}{code}|{encoded}")
}

/// Convenience for the common no-argument case.
pub fn simple(code: &str) -> anyhow::Error {
    error(code, &[])
}

/// Recognise a coded error, returning the code and its arguments. Used by tests
/// and by callers that need to react to a specific failure.
pub fn parse(message: &str) -> Option<(&str, Vec<Value>)> {
    let rest = message.strip_prefix(PREFIX)?;
    let (code, args) = rest.split_once('|')?;
    let args = serde_json::from_str(args).ok()?;
    Some((code, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_survive_the_anyhow_display_boundary() {
        let err = error("spawn_failed", &["ollama".into(), "denied".into()]);
        let message = format!("{err:#}");
        let (code, args) = parse(&message).expect("coded error");
        assert_eq!(code, "spawn_failed");
        assert_eq!(args, vec![Value::from("ollama"), Value::from("denied")]);
    }

    #[test]
    fn plain_errors_are_not_mistaken_for_codes() {
        assert!(parse("provider HTTP 500").is_none());
    }
}
