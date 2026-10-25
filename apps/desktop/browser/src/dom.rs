//! DOM snapshot helper.
//!
//! The Desktop build keeps DOM extraction inside the page's own JavaScript
//! context (see `run_js` in the Chromium backend) so no separate HTML parser is
//! needed. This module provides the selector-escaping helper the JS snippets
//! share.

/// Escape a Rust string for safe embedding inside a JavaScript single-quoted
/// literal.
pub fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

/// Build the `document.querySelector(sel) !== null` probe used by `wait_for`.
pub fn selector_probe(selector: &str) -> String {
    format!("document.querySelector({}) !== null", js_string(selector))
}

/// Build the innerHTML extractor used by the `dom` action.
pub fn inner_html_script(selector: Option<&str>) -> String {
    let sel = js_string(selector.unwrap_or_default());
    format!(
        r#"(() => {{
          const sel = {sel};
          const root = sel ? document.querySelector(sel) : document.body;
          if (!root) return null;
          return root.innerHTML;
        }})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes() {
        assert_eq!(js_string("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn probe_uses_json_encoding() {
        let probe = selector_probe("input[name=\"q\"]");
        assert!(probe.contains("input[name=\\\"q\\\"]"));
    }
}
