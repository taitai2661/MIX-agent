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
///
/// The cap is applied inside the page so a giant document never crosses CDP in
/// the first place. The backend applies the same cap again as a belt-and-braces
/// check for callers that construct the action directly.
pub fn inner_html_script(selector: Option<&str>, max_chars: usize) -> String {
    let sel = js_string(selector.unwrap_or_default());
    format!(
        r#"(() => {{
          const sel = {sel};
          const root = sel ? document.querySelector(sel) : document.body;
          if (!root) return null;
          return root.innerHTML.slice(0, {max_chars});
        }})()"#
    )
}

/// Build an accessibility-oriented element tree. Refs are stable only for the
/// current DOM generation: every snapshot clears the previous `data-mix-ref`
/// attributes before assigning new ones, so a ref can never point at a stale
/// element after a navigation.
pub fn snapshot_script(max_nodes: usize) -> String {
    format!(
        r#"(() => {{
          const max = {max_nodes};
          document.querySelectorAll('[data-mix-ref]').forEach(el => el.removeAttribute('data-mix-ref'));
          const isVisible = (el) => {{
            const s = getComputedStyle(el);
            return s.display !== 'none' && s.visibility !== 'hidden' && s.opacity !== '0' && el.getClientRects().length > 0;
          }};
          const implicitRole = (el) => {{
            const tag = el.tagName.toLowerCase();
            if (tag === 'a') return 'link';
            if (tag === 'button') return 'button';
            if (tag === 'input') {{
              const type = (el.getAttribute('type') || 'text').toLowerCase();
              if (type === 'checkbox') return 'checkbox';
              if (type === 'radio') return 'radio';
              if (type === 'submit' || type === 'button') return 'button';
              return 'textbox';
            }}
            if (tag === 'select') return 'combobox';
            if (tag === 'textarea') return 'textbox';
            if (tag === 'img') return 'img';
            if (/^h[1-6]$/.test(tag)) return 'heading';
            if (tag === 'summary') return 'button';
            return null;
          }};
          const nameOf = (el) => {{
            const raw = el.getAttribute('aria-label')
              || el.getAttribute('alt')
              || el.getAttribute('placeholder')
              || el.getAttribute('title')
              || (el.innerText || el.textContent || '');
            return String(raw).trim().replace(/\s+/g, ' ').slice(0, 160);
          }};
          const nodes = [];
          const candidates = document.querySelectorAll('a,button,input,select,textarea,summary,img,h1,h2,h3,h4,h5,h6,[role],[tabindex],[contenteditable="true"]');
          let seq = 0;
          for (const el of candidates) {{
            if (nodes.length >= max) break;
            if (!isVisible(el)) continue;
            const tag = el.tagName.toLowerCase();
            const role = el.getAttribute('role') || implicitRole(el);
            const name = nameOf(el);
            if (!role && !name) continue;
            const ref = 'r' + (++seq);
            el.setAttribute('data-mix-ref', ref);
            const rect = el.getBoundingClientRect();
            nodes.push({{
              ref,
              role: role || tag,
              name,
              tag,
              type: el.getAttribute('type') || null,
              value: (tag === 'input' || tag === 'textarea' || tag === 'select') ? String(el.value || '').slice(0, 200) : null,
              checked: tag === 'input' ? !!el.checked : null,
              disabled: !!el.disabled,
              x: Math.round(rect.x),
              y: Math.round(rect.y),
              width: Math.round(rect.width),
              height: Math.round(rect.height)
            }});
          }}
          return nodes;
        }})()"#
    )
}

/// Structured text/link extraction used by `extract`.
pub fn extract_script(selector: &str, limit: usize) -> String {
    let sel = js_string(selector);
    format!(
        r#"(() => {{
          const nodes = Array.from(document.querySelectorAll({sel})).slice(0, {limit});
          return nodes.map(el => {{
            const anchor = el.tagName.toLowerCase() === 'a' ? el : el.querySelector('a');
            return {{
              tag: el.tagName.toLowerCase(),
              text: String(el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 2000),
              href: anchor ? anchor.href : null
            }};
          }});
        }})()"#
    )
}

/// Wait until the page body contains `text`.
pub fn wait_for_text_script(text: &str) -> String {
    let text = js_string(text);
    format!(r#"(() => ((document.body && document.body.innerText) || '').includes({text}))()"#)
}

/// Wait until the document reports complete.
pub fn wait_for_load_script() -> String {
    "document.readyState === 'complete'".to_string()
}

/// Clear an input/textarea/contenteditable and focus it.
pub const CLEAR_FIELD_JS: &str = r#"function() {
  if ('value' in this) {
    this.value = '';
    this.dispatchEvent(new Event('input', { bubbles: true }));
    this.dispatchEvent(new Event('change', { bubbles: true }));
  } else {
    this.textContent = '';
    this.dispatchEvent(new InputEvent('input', { bubbles: true }));
  }
  this.focus();
  return true;
}"#;

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
