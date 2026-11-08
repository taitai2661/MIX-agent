//! JavaScript snippets used by the Tauri child-WebView browser host.
//!
//! The page is untrusted. Every snippet runs through `eval_with_callback` from
//! Rust and returns a JSON object; the result is parsed and capped before it is
//! handed back to the agent. This is the native-WebView replacement for the old
//! CDP `Runtime.evaluate` / DOM helpers.

use mixd_browser::dom;
use serde_json::Value;

pub fn js_string(value: &str) -> String {
    dom::js_string(value)
}

/// Resolve a selector or a snapshot ref (`r12`) to a JS expression.
fn element_expr(selector: &str, reference: Option<&str>) -> String {
    if let Some(reference) = reference {
        let valid = reference
            .strip_prefix('r')
            .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()));
        if !valid {
            return "null".to_string();
        }
        let selector = format!("[data-mix-ref=\"{reference}\"]");
        format!("document.querySelector({})", js_string(&selector))
    } else {
        format!("document.querySelector({})", js_string(selector))
    }
}

/// Wrap a statement body so it returns `{ ok, value }` or `{ ok:false, error }`.
pub fn wrap(body: &str) -> String {
    format!(
        r#"(() => {{
          try {{
            const value = (() => {{ {body} }})();
            return JSON.stringify({{ ok: true, value: value === undefined ? null : value }});
          }} catch (error) {{
            return JSON.stringify({{ ok: false, error: String((error && error.message) || error) }});
          }}
        }})()"#
    )
}

pub fn click(selector: &str, reference: Option<&str>) -> String {
    let element = element_expr(selector, reference);
    wrap(&format!(
        r#"
        const el = {element};
        if (!el) throw new Error('selector_not_found');
        el.scrollIntoView({{ block: 'center', inline: 'center' }});
        const rect = el.getBoundingClientRect();
        const x = rect.left + rect.width / 2;
        const y = rect.top + rect.height / 2;
        for (const type of ['pointerover','pointerenter','pointerdown','mousedown','pointerup','mouseup']) {{
          el.dispatchEvent(new MouseEvent(type, {{ bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0, view: window }}));
        }}
        if (typeof el.click === 'function') el.click();
        return {{ selector: {selector_literal}, x: Math.round(x), y: Math.round(y) }};
        "#,
        element = element,
        selector_literal = js_string(if reference.is_some() { "" } else { selector })
    ))
}

pub fn click_at(x: i32, y: i32) -> String {
    wrap(&format!(
        r#"
        const el = document.elementFromPoint({x}, {y});
        if (!el) throw new Error('element_not_found');
        el.dispatchEvent(new MouseEvent('click', {{ bubbles: true, cancelable: true, clientX: {x}, clientY: {y}, button: 0, view: window }}));
        if (typeof el.click === 'function') el.click();
        return {{ x: {x}, y: {y} }};
        "#
    ))
}

pub fn type_text(selector: &str, reference: Option<&str>, text: &str, clear: bool) -> String {
    let element = element_expr(selector, reference);
    wrap(&format!(
        r#"
        const el = {element};
        if (!el) throw new Error('selector_not_found');
        el.scrollIntoView({{ block: 'center', inline: 'center' }});
        el.focus();
        const value = {text};
        if (el.isContentEditable) {{
          if ({clear}) el.textContent = '';
          el.textContent = (el.textContent || '') + value;
          el.dispatchEvent(new InputEvent('input', {{ bubbles: true, data: value, inputType: 'insertText' }}));
        }} else {{
          const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
          const setter = Object.getOwnPropertyDescriptor(proto, 'value');
          if ({clear}) {{
            if (setter && setter.set) setter.set.call(el, ''); else el.value = '';
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
          }}
          if (setter && setter.set) setter.set.call(el, (el.value || '') + value); else el.value = (el.value || '') + value;
          el.dispatchEvent(new Event('input', {{ bubbles: true }}));
          el.dispatchEvent(new Event('change', {{ bubbles: true }}));
        }}
        return {{ typed: value.length }};
        "#,
        element = element,
        text = js_string(text),
        clear = if clear { "true" } else { "false" }
    ))
}

pub fn type_active(text: &str) -> String {
    wrap(&format!(
        r#"
        const el = document.activeElement || document.body;
        if (!el) throw new Error('no_active_element');
        const value = {text};
        if (el.isContentEditable) {{
          el.textContent = (el.textContent || '') + value;
          el.dispatchEvent(new InputEvent('input', {{ bubbles: true, data: value, inputType: 'insertText' }}));
        }} else {{
          const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
          const setter = Object.getOwnPropertyDescriptor(proto, 'value');
          if (setter && setter.set) setter.set.call(el, (el.value || '') + value); else el.value = (el.value || '') + value;
          el.dispatchEvent(new Event('input', {{ bubbles: true }}));
          el.dispatchEvent(new Event('change', {{ bubbles: true }}));
        }}
        return {{ typed: value.length }};
        "#,
        text = js_string(text)
    ))
}

pub fn press(key: &str) -> String {
    wrap(&format!(
        r#"
        const el = document.activeElement || document.body;
        const key = {key};
        const init = {{ key: key, code: key, bubbles: true, cancelable: true }};
        el.dispatchEvent(new KeyboardEvent('keydown', init));
        el.dispatchEvent(new KeyboardEvent('keypress', init));
        el.dispatchEvent(new KeyboardEvent('keyup', init));
        if (key === 'Enter' && el.form && typeof el.form.requestSubmit === 'function') el.form.requestSubmit();
        return {{ key: key }};
        "#,
        key = js_string(key)
    ))
}

pub fn hover(selector: &str, reference: Option<&str>) -> String {
    let element = element_expr(selector, reference);
    wrap(&format!(
        r#"
        const el = {element};
        if (!el) throw new Error('selector_not_found');
        el.scrollIntoView({{ block: 'center', inline: 'center' }});
        const rect = el.getBoundingClientRect();
        const x = rect.left + rect.width / 2;
        const y = rect.top + rect.height / 2;
        for (const type of ['pointerover','pointerenter','mouseover','mouseenter','mousemove']) {{
          el.dispatchEvent(new MouseEvent(type, {{ bubbles: true, cancelable: true, clientX: x, clientY: y, view: window }}));
        }}
        return {{ hovered: true }};
        "#,
        element = element
    ))
}

pub fn select(selector: &str, reference: Option<&str>, value: &str) -> String {
    let element = element_expr(selector, reference);
    wrap(&format!(
        r#"
        const el = {element};
        if (!el) throw new Error('selector_not_found');
        el.value = {value};
        el.dispatchEvent(new Event('input', {{ bubbles: true }}));
        el.dispatchEvent(new Event('change', {{ bubbles: true }}));
        return {{ value: el.value }};
        "#,
        element = element,
        value = js_string(value)
    ))
}

pub fn check(selector: &str, reference: Option<&str>, checked: bool) -> String {
    let element = element_expr(selector, reference);
    wrap(&format!(
        r#"
        const el = {element};
        if (!el) throw new Error('selector_not_found');
        if (el.checked !== {checked}) el.click();
        return {{ checked: !!el.checked }};
        "#,
        element = element,
        checked = if checked { "true" } else { "false" }
    ))
}

pub fn scroll(x: i32, y: i32) -> String {
    wrap(&format!(
        "window.scrollBy({x}, {y}); return {{ scrollY: window.scrollY, scrollX: window.scrollX }};"
    ))
}

pub fn dom_html(selector: Option<&str>) -> String {
    let sel = js_string(selector.unwrap_or_default());
    wrap(&format!(
        r#"
        const sel = {sel};
        const root = sel ? document.querySelector(sel) : document.body;
        if (!root) throw new Error('selector_not_found');
        return {{ html: root.innerHTML.slice(0, {max}) }};
        "#,
        max = mixd_browser::MAX_DOM_CHARS
    ))
}

pub fn extract(selector: &str, limit: usize) -> String {
    dom::extract_script(selector, limit)
}

pub fn snapshot(max_nodes: usize) -> String {
    dom::snapshot_script(max_nodes)
}

pub fn info() -> String {
    wrap(
        r#"
        return {
          title: document.title || null,
          url: location.href,
          readyState: document.readyState,
          canGoBack: window.history.length > 1,
          canGoForward: false
        };
        "#,
    )
}

pub fn evaluate(script: &str) -> String {
    wrap(&format!(
        r#"
        const source = {script};
        try {{ return (new Function('return (' + source + ')'))(); }}
        catch (_) {{ return new Function(source)(); }}
        "#,
        script = js_string(script)
    ))
}

pub fn selector_probe(selector: &str) -> String {
    dom::selector_probe(selector)
}

pub fn text_probe(text: &str) -> String {
    dom::wait_for_text_script(text)
}

pub fn load_probe() -> String {
    dom::wait_for_load_script()
}

pub fn url_probe(url: &str) -> String {
    wrap(&format!("return location.href === {};", js_string(url)))
}

pub fn function_probe(script: &str) -> String {
    wrap(&format!(
        r#"
        const source = {script};
        try {{ return !!(new Function('return (' + source + ')'))(); }}
        catch (_) {{ return !!new Function(source)(); }}
        "#,
        script = js_string(script)
    ))
}

pub fn reload() -> String {
    wrap("location.reload(); return true;")
}

pub fn back() -> String {
    wrap("history.back(); return true;")
}

pub fn forward() -> String {
    wrap("history.forward(); return true;")
}

pub fn stop() -> String {
    wrap("window.stop(); return true;")
}

/// Best-effort subresource guard. This is not a replacement for a network
/// proxy: it covers fetch/XHR/WebSocket/EventSource/beacon, not images, CSS,
/// iframes or workers. The navigation gate in Rust is authoritative for
/// top-level documents.
pub fn egress_guard(allow_local: bool) -> String {
    format!(
        r#"
        (() => {{
          const allowLocal = {allow_local};
          const isPrivate = (host) => {{
            if (!host) return false;
            const h = host.toLowerCase();
            if (h === 'localhost' || h.endsWith('.localhost')) return true;
            if (h === '127.0.0.1' || h === '0.0.0.0' || h === '[::1]' || h === '::1') return true;
            if (h === '169.254.169.254' || h.startsWith('169.254.')) return true;
            if (/^10\./.test(h) || /^192\.168\./.test(h)) return true;
            if (/^172\.(1[6-9]|2\d|3[01])\./.test(h)) return true;
            return false;
          }};
          const allowed = (url) => {{
            try {{
              const u = new URL(url, location.href);
              if (u.protocol !== 'http:' && u.protocol !== 'https:') return true;
              return allowLocal || !isPrivate(u.hostname);
            }} catch (_) {{ return true; }}
          }};
          const fail = () => Promise.reject(new TypeError('blocked by MIX egress policy'));
          if (window.fetch) {{
            const original = window.fetch;
            window.fetch = function(input, init) {{
              const url = typeof input === 'string' ? input : (input && input.url) || '';
              return allowed(url) ? original.call(this, input, init) : fail();
            }};
          }}
          if (window.XMLHttpRequest) {{
            const open = XMLHttpRequest.prototype.open;
            XMLHttpRequest.prototype.open = function(method, url, ...rest) {{
              if (!allowed(String(url))) throw new TypeError('blocked by MIX egress policy');
              return open.call(this, method, url, ...rest);
            }};
          }}
          for (const name of ['WebSocket', 'EventSource']) {{
            const Original = window[name];
            if (!Original) continue;
            window[name] = class extends Original {{
              constructor(url, ...rest) {{
                if (!allowed(String(url))) throw new TypeError('blocked by MIX egress policy');
                super(url, ...rest);
              }}
            }};
          }}
          if (navigator.sendBeacon) {{
            const beacon = navigator.sendBeacon.bind(navigator);
            navigator.sendBeacon = (url, data) => allowed(String(url)) ? beacon(url, data) : false;
          }}
        }})();
        "#,
        allow_local = if allow_local { "true" } else { "false" }
    )
}

/// Parse the callback value returned by `Webview::eval_with_callback`.
///
/// Wry serialises the JavaScript result as JSON. Our snippets return a JSON
/// *string*, so the callback normally carries a quoted string and needs one
/// more parse.
pub fn parse_result(raw: &str) -> anyhow::Result<Value> {
    let first: Value = serde_json::from_str(raw)
        .map_err(|err| anyhow::anyhow!("invalid JS result: {err}; raw={raw}"))?;
    let value = match first {
        Value::String(inner) => {
            let trimmed = inner.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                serde_json::from_str(&inner).unwrap_or(Value::String(inner))
            } else {
                Value::String(inner)
            }
        }
        other => other,
    };
    if let Some(ok) = value.get("ok").and_then(Value::as_bool) {
        if ok {
            Ok(value.get("value").cloned().unwrap_or(Value::Null))
        } else {
            let message = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("browser script failed");
            Err(anyhow::anyhow!("{message}"))
        }
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_wrapped_action_result_from_the_callback() {
        let raw = serde_json::to_string(
            r#"{"ok":true,"value":{"url":"https://example.com/","title":"Example"}}"#,
        )
        .expect("encode callback");
        let value = parse_result(&raw).expect("parse");
        assert_eq!(value["url"], "https://example.com/");
    }

    #[test]
    fn parses_a_raw_snapshot_array() {
        let value = parse_result(r#"[{"ref":"r1","role":"button"}]"#).expect("parse");
        assert_eq!(value[0]["ref"], "r1");
    }

    #[test]
    fn surfaces_a_script_error() {
        let raw = serde_json::to_string(r#"{"ok":false,"error":"selector_not_found"}"#)
            .expect("encode callback");
        let error = parse_result(&raw).expect_err("must fail");
        assert!(error.to_string().contains("selector_not_found"));
    }

    #[test]
    fn rejects_invalid_refs_before_they_reach_the_selector() {
        let valid = element_expr("", Some("r12"));
        assert!(valid.contains("data-mix-ref"));
        assert!(valid.contains("r12"));
        assert_eq!(element_expr("", Some("../../x")), "null");
    }
}
