//! Secret masking for tool output.
//!
//! The agent reads `.env` files, runs `env`, and fetches pages; any of those can
//! put a live credential into the model context, the SQLite transcript and the
//! event log at once. The values are not needed to do the task, so they are
//! replaced with a marker before the result leaves the tool boundary.
//!
//! This is defence in depth, not a guarantee: a key pasted with no recognisable
//! shape or name cannot be identified. It deliberately errs towards masking
//! rather than towards leaking.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

/// Replacement text. Kept short so it cannot itself look like a credential.
pub const MASK: &str = "[redacted]";

/// `NAME=value` / `"name": "value"` where the name says it is a secret.
const ASSIGNMENT: &str = r#"(?i)([A-Za-z0-9_.-]*(?:api[_-]?key|token|secret|password|passwd|access[_-]?key|private[_-]?key|client[_-]?secret)[A-Za-z0-9_.-]*\s*[:=]\s*)"?(?-i)([A-Za-z0-9_\-\.\+/=]{8,})"#;

/// Well-known credential prefixes. Anything matching is masked even when the
/// variable/field name gives nothing away.
const SHAPES: &[&str] = &[
    r"sk-[A-Za-z0-9_-]{16,}",
    r"sk-ant-[A-Za-z0-9_-]{16,}",
    r"ghp_[A-Za-z0-9]{20,}",
    r"gho_[A-Za-z0-9]{20,}",
    r"github_pat_[A-Za-z0-9_]{20,}",
    r"xox[baprs]-[A-Za-z0-9-]{10,}",
    r"AKIA[0-9A-Z]{16}",
    r"AIza[0-9A-Za-z_-]{35}",
    r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
];

fn assignment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(ASSIGNMENT).expect("assignment regex"))
}

fn shapes() -> &'static [Regex] {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        SHAPES
            .iter()
            .filter_map(|pattern| Regex::new(pattern).ok())
            .collect()
    })
}

/// Live values of secret-looking environment variables, longest first so a
/// value that contains another is masked as a whole.
fn environment_values() -> &'static [String] {
    static VALUES: OnceLock<Vec<String>> = OnceLock::new();
    VALUES.get_or_init(|| {
        let mut values: Vec<String> = std::env::vars()
            .filter(|(name, value)| {
                let upper = name.to_ascii_uppercase();
                value.len() >= 8
                    && (upper.contains("KEY")
                        || upper.contains("TOKEN")
                        || upper.contains("SECRET")
                        || upper.contains("PASSWORD"))
                    && !name.starts_with("MIX_")
            })
            .map(|(_, value)| value)
            .collect();
        values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        values
    })
}

/// Mask credentials in free text.
pub fn redact_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = assignment()
        .replace_all(text, |caps: &regex::Captures<'_>| {
            format!("{}{}", &caps[1], MASK)
        })
        .into_owned();
    for pattern in shapes() {
        out = pattern.replace_all(&out, MASK).into_owned();
    }
    for value in environment_values() {
        if out.contains(value.as_str()) {
            out = out.replace(value.as_str(), MASK);
        }
    }
    out
}

/// Mask every string in a tool result payload, in place.
pub fn redact_value(value: &mut Value) {
    match value {
        Value::String(text) => {
            let masked = redact_text(text);
            if masked != *text {
                *text = masked;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact_value),
        Value::Object(map) => map.values_mut().for_each(redact_value),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_shapes_are_masked() {
        let text = "OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz\npassword: hunter2hunter2\n";
        let masked = redact_text(text);
        assert!(!masked.contains("sk-abcdefghijklmnopqrstuvwxyz"));
        assert!(!masked.contains("hunter2hunter2"));
        assert!(masked.contains("OPENAI_API_KEY="));
        assert!(masked.contains(MASK));
    }

    #[test]
    fn known_token_shapes_are_masked_without_a_name() {
        let masked = redact_text("here is ghp_abcdefghijklmnopqrstuvwxyz0123456789 ok");
        assert!(!masked.contains("ghp_"));
        assert!(masked.contains(MASK));
    }

    #[test]
    fn ordinary_code_is_left_alone() {
        let text = "let count: usize = 42;\nfn main() { println!(\"hi\"); }";
        assert_eq!(redact_text(text), text);
    }

    #[test]
    fn nested_payloads_are_masked() {
        let mut value = serde_json::json!({
            "stdout": "GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "files": ["sk-abcdefghijklmnopqrstuvwxyz"],
        });
        redact_value(&mut value);
        let rendered = value.to_string();
        assert!(!rendered.contains("ghp_"));
        assert!(!rendered.contains("sk-abcdefghijklmnopqrstuvwxyz"));
    }
}
