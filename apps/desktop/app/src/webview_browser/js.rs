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

/// Best-effort subresource guard injected into the untrusted page.
///
/// This is NOT a security boundary and NOT a replacement for a network proxy.
/// It patches the JavaScript network/load sinks we can see (fetch, XHR,
/// WebSocket, EventSource, beacon, worker registration, URL-bearing element
/// attributes, `setAttribute`, CSS `url()`/`insertRule`, plus a mutation pass
/// for DOM-created subresources) and makes the replacements non-configurable
/// where the engine allows it. A page that captured an original before this
/// script ran, defined an own property on an element, or navigates through a
/// sink we missed can still make a request; none of that is prevented here.
///
/// The Rust `on_navigation` gate (`host.rs`) is the authoritative check for
/// **top-level** documents only — wry does not expose a subresource request
/// hook, so there is nothing to wire at the WebView level. Loopback blocking
/// for subresources therefore relies on this shim and must be treated as a
/// speed bump, not a guarantee.
pub fn egress_guard(allow_local: bool) -> String {
    // A sentinel replacement keeps the large script readable: `format!` would
    // require doubling every JS brace.
    const SHIM: &str = r#"(() => {
      const initial = __MIX_ALLOW_LOCAL__;
      // `set_allow_local` re-evaluates this snippet on live tabs. Wrappers are
      // installed once; a later call only flips the shared flag.
      if (window.__mixEgressGuard) {
        window.__mixEgressGuard.allowLocal = initial;
        return;
      }
      const guard = { allowLocal: initial };
      try {
        Object.defineProperty(window, '__mixEgressGuard', {
          value: guard, writable: false, configurable: false,
        });
      } catch (_) {
        window.__mixEgressGuard = guard;
      }

      // Keep this list in sync with `mixd_browser::is_forbidden_ip` where
      // possible; it is deliberately conservative (blocks link-local and
      // .local/.internal) because a false positive only breaks a subresource.
      const isPrivate = (host) => {
        if (!host) return false;
        const h = String(host).toLowerCase();
        if (h === 'localhost' || h.endsWith('.localhost')) return true;
        if (h === '127.0.0.1' || h === '0.0.0.0' || h === '[::1]' || h === '::1') return true;
        if (h === '::ffff:127.0.0.1' || h === '0:0:0:0:0:0:0:1') return true;
        if (h === '169.254.169.254' || h.startsWith('169.254.')) return true;
        if (/^10\./.test(h) || /^192\.168\./.test(h)) return true;
        if (/^172\.(1[6-9]|2\d|3[01])\./.test(h)) return true;
        if (h.endsWith('.local') || h.endsWith('.internal')) return true;
        return false;
      };
      const allowed = (raw) => {
        try {
          const u = new URL(String(raw), location.href);
          // data:, blob:, about: etc. do not open a network connection here.
          if (u.protocol !== 'http:' && u.protocol !== 'https:') return true;
          return guard.allowLocal || !isPrivate(u.hostname);
        } catch (_) {
          // Fail closed: an unexpected parse error must not turn into a
          // permission to open a network connection.
          return false;
        }
      };
      const reason = 'blocked by MIX egress policy';
      const fail = () => Promise.reject(new TypeError(reason));
      const deny = () => { throw new TypeError(reason); };
      const cssBlocked = (text) => {
        const re = /url\(\s*(['"]?)([^'")]+)\1\s*\)/gi;
        let match;
        while ((match = re.exec(String(text)))) {
          if (!allowed(match[2])) return true;
        }
        return false;
      };
      // Inspect an HTML string *before* the parser can start fetching the
      // subresources it contains. The MutationObserver below is reactive: by
      // the time it removes a blocked attribute, `<img src=…>` may already
      // have hit the network.
      const htmlBlocked = (html) => {
        const text = String(html);
        if (cssBlocked(text)) return true;
        const re = /(?:src|href|srcset|data|poster|formaction|action|xlink:href)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/gi;
        let match;
        while ((match = re.exec(text))) {
          const value = match[1] !== undefined ? match[1]
            : (match[2] !== undefined ? match[2] : (match[3] || ''));
          if (!allowed(value)) return true;
        }
        return false;
      };

      // Non-configurable own properties resist the common `window.fetch = f`
      // tamper. This is not bulletproof: a captured original still works.
      const install = (obj, name, value) => {
        try {
          const current = Object.getOwnPropertyDescriptor(obj, name);
          if (current && !current.configurable) return;
          Object.defineProperty(obj, name, {
            value, writable: false, configurable: false,
            enumerable: current ? current.enumerable : true,
          });
        } catch (_) {
          try { obj[name] = value; } catch (_) {}
        }
      };

      // Explicit network APIs.
      if (window.fetch) {
        const original = window.fetch;
        install(window, 'fetch', function(input, init) {
          const url = typeof input === 'string' ? input : (input && input.url) || '';
          return allowed(url) ? original.call(this, input, init) : fail();
        });
      }
      if (window.XMLHttpRequest) {
        const open = XMLHttpRequest.prototype.open;
        install(XMLHttpRequest.prototype, 'open', function(method, url, ...rest) {
          if (!allowed(String(url))) return deny();
          return open.call(this, method, url, ...rest);
        });
      }
      for (const name of ['WebSocket', 'EventSource', 'Worker', 'SharedWorker']) {
        const Original = window[name];
        if (!Original) continue;
        install(window, name, class extends Original {
          constructor(url, ...rest) {
            if (!allowed(String(url))) throw new TypeError(reason);
            super(url, ...rest);
          }
        });
      }
      if (navigator.serviceWorker && navigator.serviceWorker.register) {
        const register = navigator.serviceWorker.register.bind(navigator.serviceWorker);
        install(navigator.serviceWorker, 'register', (url, ...rest) =>
          allowed(String(url)) ? register(url, ...rest) : fail());
      }
      if (navigator.sendBeacon) {
        const beacon = navigator.sendBeacon.bind(navigator);
        install(navigator, 'sendBeacon', (url, data) =>
          allowed(String(url)) ? beacon(url, data) : false);
      }

      // Subresources that never touch fetch: <img>, <iframe>, <link>,
      // <script>, <video poster>, <object data>, form actions, ...
      const URL_ATTRS = ['src', 'href', 'srcset', 'data', 'poster', 'formaction', 'action', 'xlink:href'];
      const guardSetter = (proto, name) => {
        try {
          const desc = Object.getOwnPropertyDescriptor(proto, name);
          if (!desc || !desc.set || !desc.configurable) return;
          Object.defineProperty(proto, name, {
            get: desc.get,
            set: function(value) {
              if (!allowed(value)) return;
              desc.set.call(this, value);
            },
            configurable: false,
            enumerable: desc.enumerable,
          });
        } catch (_) {}
      };
      for (const [ctor, props] of [
        ['HTMLImageElement', ['src', 'srcset']],
        ['HTMLIFrameElement', ['src']],
        ['HTMLFrameElement', ['src']],
        ['HTMLLinkElement', ['href']],
        ['HTMLScriptElement', ['src']],
        ['HTMLMediaElement', ['src', 'poster']],
        ['HTMLSourceElement', ['src', 'srcset']],
        ['HTMLTrackElement', ['src']],
        ['HTMLObjectElement', ['data']],
        ['HTMLEmbedElement', ['src']],
        ['HTMLInputElement', ['src', 'formaction']],
        ['HTMLButtonElement', ['formaction']],
        ['HTMLFormElement', ['action']],
      ]) {
        const proto = window[ctor] && window[ctor].prototype;
        if (!proto) continue;
        for (const prop of props) guardSetter(proto, prop);
      }
      if (window.Element && Element.prototype.setAttribute) {
        const setAttribute = Element.prototype.setAttribute;
        install(Element.prototype, 'setAttribute', function(name, value) {
          const key = String(name).toLowerCase().replace(/^.*:/, '');
          if (URL_ATTRS.includes(key) && !allowed(value)) return;
          if (key === 'style' && cssBlocked(value)) return;
          return setAttribute.call(this, name, value);
        });
        const setAttributeNS = Element.prototype.setAttributeNS;
        if (setAttributeNS) {
          install(Element.prototype, 'setAttributeNS', function(ns, name, value) {
            const key = String(name).toLowerCase().replace(/^.*:/, '');
            if (URL_ATTRS.includes(key) && !allowed(value)) return;
            return setAttributeNS.call(this, ns, name, value);
          });
        }
      }
      // Filter markup before it is parsed. Without these, innerHTML / JSX
      // injection of `<img src="http://169.254.169.254/…">` could fire a
      // request before the reactive attribute scanner noticed the node.
      if (window.Element && Element.prototype) {
        for (const name of ['innerHTML', 'outerHTML']) {
          const desc = Object.getOwnPropertyDescriptor(Element.prototype, name);
          if (!desc || !desc.set || !desc.configurable) continue;
          Object.defineProperty(Element.prototype, name, {
            get: desc.get,
            set: function(value) {
              if (htmlBlocked(value)) return;
              desc.set.call(this, value);
            },
            configurable: false,
            enumerable: desc.enumerable,
          });
        }
        if (Element.prototype.insertAdjacentHTML) {
          const insert = Element.prototype.insertAdjacentHTML;
          install(Element.prototype, 'insertAdjacentHTML', function(position, html) {
            if (htmlBlocked(html)) return;
            return insert.call(this, position, html);
          });
        }
      }
      if (window.CSSStyleSheet && CSSStyleSheet.prototype.insertRule) {
        const insertRule = CSSStyleSheet.prototype.insertRule;
        install(CSSStyleSheet.prototype, 'insertRule', function(rule, index) {
          if (cssBlocked(rule)) return 0;
          return insertRule.call(this, rule, index);
        });
      }

      // innerHTML/createElement paths the setters above never see. Reactive by
      // necessity: it removes a blocked attribute after the node is inserted.
      try {
        const seen = new WeakSet();
        const scan = (node) => {
          if (!node || seen.has(node)) return;
          seen.add(node);
          if (node.nodeType === 1) {
            for (const attr of Array.from(node.attributes || [])) {
              const key = attr.name.toLowerCase().replace(/^.*:/, '');
              if (URL_ATTRS.includes(key) && !allowed(attr.value)) {
                try { node.removeAttribute(attr.name); } catch (_) {}
              }
            }
            if (node.tagName === 'STYLE' && cssBlocked(node.textContent || '')) {
              try { node.textContent = ''; } catch (_) {}
            }
          }
          for (const child of Array.from(node.children || [])) scan(child);
        };
        new MutationObserver((records) => {
          for (const record of records) {
            for (const added of record.addedNodes) scan(added);
            if (record.target) scan(record.target);
          }
        }).observe(document, { childList: true, subtree: true, attributes: true });
      } catch (_) {}
    })();"#;
    SHIM.replace(
        "__MIX_ALLOW_LOCAL__",
        if allow_local { "true" } else { "false" },
    )
}

/// Parse the callback value returned by `Webview::eval_with_callback`.
///
/// Wry serialises the JavaScript result as JSON. Our snippets return a JSON
/// *string*, so the callback normally carries a quoted string and needs one
/// more parse.
/// Describe an unparsable JS result without echoing its contents.
///
/// `eval_with_callback` returns whatever the page last wrote, which can include
/// bearer tokens, form values or other secrets. The old error embedded the raw
/// string in logs; keep only structural metadata so a parse failure is still
/// diagnosable without copying page data into the log.
fn summarize_raw_result(raw: &str) -> String {
    let trimmed = raw.trim_start();
    let shape = if trimmed.starts_with('{') {
        "object-like"
    } else if trimmed.starts_with('[') {
        "array-like"
    } else if trimmed.starts_with('"') {
        "string-like"
    } else {
        "unrecognised"
    };
    format!("{} bytes, {shape}", raw.len())
}

pub fn parse_result(raw: &str) -> anyhow::Result<Value> {
    let first: Value = serde_json::from_str(raw).map_err(|err| {
        anyhow::anyhow!("invalid JS result: {err}; {}", summarize_raw_result(raw))
    })?;
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

    #[test]
    fn the_egress_shim_covers_subresources_and_substitutes_the_toggle() {
        let shim = egress_guard(false);
        assert!(shim.contains("const initial = false;"));
        assert!(!shim.contains("__MIX_ALLOW_LOCAL__"));
        for sink in [
            "HTMLImageElement",
            "HTMLIFrameElement",
            "HTMLLinkElement",
            "Worker",
            "SharedWorker",
            "CSSStyleSheet",
            "setAttribute",
            "MutationObserver",
            "htmlBlocked",
            "innerHTML",
            "insertAdjacentHTML",
            "sendBeacon",
            "EventSource",
            "serviceWorker",
        ] {
            assert!(shim.contains(sink), "guard missed {sink}");
        }
        assert!(egress_guard(true).contains("const initial = true;"));
    }

    #[test]
    fn a_parse_failure_summarises_without_echoing_page_secrets() {
        let error = parse_result("Bearer super-secret-token, not JSON").unwrap_err();
        let message = error.to_string();
        assert!(!message.contains("super-secret"), "{message}");
        assert!(message.contains("bytes"), "{message}");
    }
}
