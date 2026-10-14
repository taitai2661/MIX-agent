//! Healing for leaked DeepSeek DSML tool-call markup.
//!
//! DeepSeek V3.2/V4 models emit tool calls as DSML markup which the provider
//! is supposed to parse into structured `tool_calls`. Intermittently —
//! especially with `tool_choice=auto` + streaming, or when a parameter value
//! contains newlines — the raw markup leaks into the assistant `content`
//! instead:
//!
//! ```text
//! <｜DSML｜tool_calls>
//! <｜DSML｜invoke name="shell">
//! <｜DSML｜parameter name="command" string="true">ls</｜DSML｜parameter>
//! </｜DSML｜invoke>
//! </｜DSML｜tool_calls>
//! ```
//!
//! Without healing the turn looks like plain text: no tool runs, the raw
//! `<｜DSML｜…>` tags reach the transcript, and the run silently stalls.
//! This module parses leaked DSML back into structured tool calls and strips
//! the markup from the visible text. The `｜` (U+FF5C) delimiters are
//! DeepSeek-internal tokens that never appear in legitimate output, so
//! healing is always on.
//!
//! Both delimiter flavours are handled: `<｜DSML｜…>` (V3.2 / V4 Pro) and
//! `<｜｜DSML｜｜…>` (V4 Flash, doubled delimiter with self-closing `invoke`
//! parameters). ASCII fallbacks (`<|DSML|…>`, `<||DSML||…>`) are accepted too
//! so a normaliser that folded the fullwidth bars can never make healing a
//! no-op.

use std::collections::BTreeMap;

/// One delimiter flavour of the DSML markup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DsmlDialect {
    /// Prefix common to every opening tag of this dialect.
    marker: &'static str,
    invoke_open: &'static str,
    invoke_close: &'static str,
    calls_open: &'static str,
    calls_close: &'static str,
}

/// `<｜DSML｜…>` — V3.2 / V4 Pro.
const SINGLE_BAR: DsmlDialect = DsmlDialect {
    marker: "<｜DSML｜",
    invoke_open: "<｜DSML｜invoke",
    invoke_close: "</｜DSML｜invoke>",
    calls_open: "<｜DSML｜tool_calls>",
    calls_close: "</｜DSML｜tool_calls>",
};

/// `<｜｜DSML｜｜…>` — V4 Flash.
const DOUBLE_BAR: DsmlDialect = DsmlDialect {
    marker: "<｜｜DSML｜｜",
    invoke_open: "<｜｜DSML｜｜invoke",
    invoke_close: "</｜｜DSML｜｜invoke>",
    calls_open: "<｜｜DSML｜｜tool_calls>",
    calls_close: "</｜｜DSML｜｜tool_calls>",
};

/// `<|DSML|…>` — ASCII-folded single bar.
const ASCII_SINGLE: DsmlDialect = DsmlDialect {
    marker: "<|DSML|",
    invoke_open: "<|DSML|invoke",
    invoke_close: "</|DSML|invoke>",
    calls_open: "<|DSML|tool_calls>",
    calls_close: "</|DSML|tool_calls>",
};

/// `<||DSML||…>` — ASCII-folded double bar.
const ASCII_DOUBLE: DsmlDialect = DsmlDialect {
    marker: "<||DSML||",
    invoke_open: "<||DSML||invoke",
    invoke_close: "</||DSML||invoke>",
    calls_open: "<||DSML||tool_calls>",
    calls_close: "</||DSML||tool_calls>",
};

/// More specific (longer) markers first so detection prefers them.
const DIALECTS: [DsmlDialect; 4] = [DOUBLE_BAR, ASCII_DOUBLE, SINGLE_BAR, ASCII_SINGLE];

fn is_double_bar(dialect: DsmlDialect) -> bool {
    dialect == DOUBLE_BAR || dialect == ASCII_DOUBLE
}

/// The dialect whose marker appears earliest in `text`, if any.
fn detect_dialect(text: &str) -> Option<(DsmlDialect, usize)> {
    DIALECTS
        .iter()
        .filter_map(|dialect| text.find(dialect.marker).map(|at| (*dialect, at)))
        .min_by_key(|(_, at)| *at)
}

/// True when the text contains any DSML marker.
pub fn contains_dsml(text: &str) -> bool {
    detect_dialect(text).is_some()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DsmlToolCall {
    pub name: String,
    /// JSON-encoded arguments object, ready for `function.arguments`.
    pub arguments: String,
}

/// Parse leaked DSML tool-call markup out of `text`.
///
/// Returns the cleaned visible text plus the parsed calls, or `None` when no
/// complete `<…invoke>` block could be parsed (the text is then left
/// untouched so nothing is lost).
pub fn parse_leaked_tool_calls(text: &str) -> Option<(String, Vec<DsmlToolCall>)> {
    let mut calls = Vec::new();
    let mut cleaned = String::new();
    let mut copied_through = 0;
    let mut search_from = 0;

    while let Some((dialect, start)) = find_next_invoke(text, search_from) {
        match parse_invoke(text, start, dialect) {
            Ok((end, call)) => {
                cleaned.push_str(&text[copied_through..start]);
                calls.push(call);
                copied_through = end;
                search_from = end;
            }
            Err(()) => {
                // Preserve malformed candidates byte-for-byte, but continue so
                // an unrelated valid call later in the message can still heal.
                let Some(end) = malformed_invoke_end(text, start, dialect) else {
                    break;
                };
                search_from = end;
            }
        }
    }

    if calls.is_empty() {
        return None;
    }
    cleaned.push_str(&text[copied_through..]);
    Some((strip_empty_envelopes(cleaned), calls))
}

#[derive(Debug)]
struct ParsedTag {
    name: String,
    attrs: BTreeMap<String, String>,
    self_closing: bool,
    end: usize,
}

fn find_next_invoke(text: &str, from: usize) -> Option<(DsmlDialect, usize)> {
    if from >= text.len() {
        return None;
    }
    DIALECTS
        .iter()
        .filter_map(|dialect| {
            let mut offset = from;
            while let Some(found) = text[offset..].find(dialect.invoke_open) {
                let start = offset + found;
                let after = start + dialect.invoke_open.len();
                if text[after..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_whitespace() || ch == '>')
                {
                    return Some((*dialect, start));
                }
                offset = after;
            }
            None
        })
        .min_by_key(|(_, start)| *start)
}

fn parse_invoke(
    text: &str,
    start: usize,
    dialect: DsmlDialect,
) -> Result<(usize, DsmlToolCall), ()> {
    let invoke = parse_open_tag(text, start, dialect)?;
    if invoke.name != "invoke" || invoke.self_closing {
        return Err(());
    }
    let name = invoke
        .attrs
        .get("name")
        .filter(|name| !name.is_empty())
        .ok_or(())?;
    let mut arguments = serde_json::Map::new();
    let mut cursor = invoke.end;

    loop {
        cursor = skip_whitespace(text, cursor);
        if text
            .get(cursor..)
            .is_some_and(|t| t.starts_with(dialect.invoke_close))
        {
            let end = cursor + dialect.invoke_close.len();
            return Ok((
                end,
                DsmlToolCall {
                    name: name.clone(),
                    arguments: serde_json::Value::Object(arguments).to_string(),
                },
            ));
        }
        if !text
            .get(cursor..)
            .is_some_and(|t| t.starts_with(dialect.marker))
        {
            return Err(());
        }

        let parameter = parse_open_tag(text, cursor, dialect)?;
        let parameter_name = parameter
            .attrs
            .get("name")
            .filter(|name| !name.is_empty())
            .ok_or(())?
            .clone();
        if arguments.contains_key(&parameter_name) {
            return Err(());
        }

        let value =
            if is_double_bar(dialect) && parameter.name == "invoke" && parameter.self_closing {
                serde_json::Value::String(parameter.attrs.get("string").ok_or(())?.clone())
            } else if parameter.name == "parameter" && !parameter.self_closing {
                let string_kind = parameter
                    .attrs
                    .get("string")
                    .map(String::as_str)
                    .ok_or(())?;
                if !matches!(string_kind, "true" | "false") {
                    return Err(());
                }
                let close = format!("</{}parameter>", dialect.marker.trim_start_matches('<'));
                let tail = text.get(parameter.end..).ok_or(())?;
                let close_offset = tail.find(&close).ok_or(())?;
                if tail
                    .find(dialect.marker)
                    .into_iter()
                    .chain(tail.find(dialect.invoke_close))
                    .any(|offset| offset < close_offset)
                {
                    return Err(());
                }
                let close_at = parameter.end + close_offset;
                let raw = text.get(parameter.end..close_at).ok_or(())?;
                cursor = close_at + close.len();
                if string_kind == "false" {
                    serde_json::from_str(raw.trim()).map_err(|_| ())?
                } else {
                    serde_json::Value::String(raw.to_string())
                }
            } else {
                return Err(());
            };
        if parameter.self_closing {
            cursor = parameter.end;
        }
        arguments.insert(parameter_name, value);
    }
}

/// Find the balanced end of a malformed invoke so recovery never descends
/// into a nested, valid-looking call and executes it out of context.
fn malformed_invoke_end(text: &str, start: usize, dialect: DsmlDialect) -> Option<usize> {
    let outer = scan_open_tag(text, start, dialect)?;
    if outer.name != "invoke" || outer.self_closing {
        return Some(outer.end);
    }
    let mut depth = 1usize;
    let mut cursor = outer.end;
    while cursor < text.len() {
        let next_foreign_open = DIALECTS
            .iter()
            .filter(|candidate| **candidate != dialect)
            .filter_map(|candidate| {
                text[cursor..]
                    .find(candidate.marker)
                    .map(|offset| cursor + offset)
            })
            .min();
        let next_open = text[cursor..]
            .find(dialect.marker)
            .map(|offset| cursor + offset);
        let next_close = text[cursor..]
            .find(dialect.invoke_close)
            .map(|offset| cursor + offset);
        let next_current = next_open.into_iter().chain(next_close).min();
        if next_foreign_open
            .is_some_and(|foreign| next_current.is_none_or(|current| foreign < current))
        {
            return None;
        }
        match (next_open, next_close) {
            (None, Some(close)) => {
                depth -= 1;
                cursor = close + dialect.invoke_close.len();
                if depth == 0 {
                    return Some(cursor);
                }
            }
            (Some(open), Some(close)) if close < open => {
                depth -= 1;
                cursor = close + dialect.invoke_close.len();
                if depth == 0 {
                    return Some(cursor);
                }
            }
            (Some(open), _) => {
                let tag = scan_open_tag(text, open, dialect)?;
                if tag.name == "invoke" && !tag.self_closing {
                    depth += 1;
                } else if tag.name == "parameter" && !tag.self_closing {
                    let close = parameter_close(dialect);
                    let tail = text.get(tag.end..)?;
                    let close_offset = tail.find(&close)?;
                    if tail
                        .find(dialect.marker)
                        .is_some_and(|offset| offset < close_offset)
                    {
                        return None;
                    }
                    cursor = tag.end + close_offset + close.len();
                    continue;
                }
                cursor = tag.end;
            }
            (None, None) => return None,
        }
    }
    None
}

/// Locate a tag boundary without accepting its attributes.
fn scan_open_tag(text: &str, start: usize, dialect: DsmlDialect) -> Option<ParsedTag> {
    if !text
        .get(start..)
        .is_some_and(|t| t.starts_with(dialect.marker))
    {
        return None;
    }
    let mut cursor = start + dialect.marker.len();
    let name_start = cursor;
    while let Some(ch) = text[cursor..].chars().next() {
        if ch.is_whitespace() || matches!(ch, '/' | '>') {
            break;
        }
        cursor += ch.len_utf8();
    }
    if cursor == name_start {
        return None;
    }
    let name = text[name_start..cursor].to_string();
    let mut quoted = false;
    let mut escaped = false;
    while let Some(ch) = text[cursor..].chars().next() {
        cursor += ch.len_utf8();
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
        } else if ch == '"' {
            quoted = true;
        } else if ch == '>' {
            let before = text[..cursor - 1].trim_end();
            return Some(ParsedTag {
                name,
                attrs: BTreeMap::new(),
                self_closing: before.ends_with('/'),
                end: cursor,
            });
        }
    }
    None
}

fn parse_open_tag(text: &str, start: usize, dialect: DsmlDialect) -> Result<ParsedTag, ()> {
    if !text
        .get(start..)
        .is_some_and(|t| t.starts_with(dialect.marker))
    {
        return Err(());
    }
    let mut cursor = start + dialect.marker.len();
    let name_start = cursor;
    while let Some(ch) = text.get(cursor..).and_then(|t| t.chars().next()) {
        if ch.is_whitespace() || matches!(ch, '/' | '>') {
            break;
        }
        cursor += ch.len_utf8();
    }
    if cursor == name_start {
        return Err(());
    }
    let name = text.get(name_start..cursor).ok_or(())?.to_string();
    let mut attrs = BTreeMap::new();

    loop {
        cursor = skip_whitespace(text, cursor);
        match text
            .get(cursor..)
            .and_then(|t| t.chars().next())
            .ok_or(())?
        {
            '>' => {
                return Ok(ParsedTag {
                    name,
                    attrs,
                    self_closing: false,
                    end: cursor + 1,
                });
            }
            '/' => {
                cursor += 1;
                cursor = skip_whitespace(text, cursor);
                if !text.get(cursor..).is_some_and(|t| t.starts_with('>')) {
                    return Err(());
                }
                return Ok(ParsedTag {
                    name,
                    attrs,
                    self_closing: true,
                    end: cursor + 1,
                });
            }
            _ => {}
        }

        let key_start = cursor;
        while let Some(ch) = text.get(cursor..).and_then(|t| t.chars().next()) {
            if ch.is_whitespace() || ch == '=' {
                break;
            }
            if matches!(ch, '/' | '>') {
                return Err(());
            }
            cursor += ch.len_utf8();
        }
        if cursor == key_start {
            return Err(());
        }
        let key = text.get(key_start..cursor).ok_or(())?.to_string();
        cursor = skip_whitespace(text, cursor);
        if !text.get(cursor..).is_some_and(|t| t.starts_with('=')) {
            return Err(());
        }
        cursor += 1;
        cursor = skip_whitespace(text, cursor);
        if !text.get(cursor..).is_some_and(|t| t.starts_with('"')) {
            return Err(());
        }
        cursor += 1;
        let (value, end) = parse_quoted_value(text, cursor)?;
        cursor = end;
        if attrs.insert(key, value).is_some() {
            return Err(());
        }
        if text
            .get(cursor..)
            .and_then(|t| t.chars().next())
            .is_some_and(|ch| !ch.is_whitespace() && !matches!(ch, '/' | '>'))
        {
            return Err(());
        }
    }
}

fn parse_quoted_value(text: &str, mut cursor: usize) -> Result<(String, usize), ()> {
    let mut value = String::new();
    loop {
        let ch = text
            .get(cursor..)
            .and_then(|t| t.chars().next())
            .ok_or(())?;
        cursor += ch.len_utf8();
        match ch {
            '"' => return Ok((value, cursor)),
            '\\' => {
                let escaped = text
                    .get(cursor..)
                    .and_then(|t| t.chars().next())
                    .ok_or(())?;
                if matches!(escaped, '"' | '\\') {
                    value.push(escaped);
                    cursor += escaped.len_utf8();
                } else {
                    value.push('\\');
                }
            }
            other => value.push(other),
        }
    }
}

fn skip_whitespace(text: &str, mut cursor: usize) -> usize {
    while let Some(ch) = text.get(cursor..).and_then(|t| t.chars().next()) {
        if !ch.is_whitespace() {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

fn parameter_close(dialect: DsmlDialect) -> String {
    format!("</{}parameter>", dialect.marker.trim_start_matches('<'))
}

/// Remove only matched envelope pairs whose contents became empty after all
/// successfully parsed invokes were removed.
fn strip_empty_envelopes(mut text: String) -> String {
    loop {
        let mut removed = false;
        for dialect in DIALECTS {
            let mut search_from = 0;
            while let Some(open_offset) = text[search_from..].find(dialect.calls_open) {
                let open = search_from + open_offset;
                let inner = open + dialect.calls_open.len();
                let Some(close_offset) = text[inner..].find(dialect.calls_close) else {
                    break;
                };
                let close = inner + close_offset;
                if text[inner..close].trim().is_empty() {
                    text.replace_range(open..close + dialect.calls_close.len(), "");
                    removed = true;
                    break;
                }
                search_from = close + dialect.calls_close.len();
            }
            if removed {
                break;
            }
        }
        if !removed {
            return text;
        }
    }
}

pub fn synthesize_call_id() -> String {
    format!("call_dsml_{}", uuid::Uuid::new_v4().simple())
}

/// Display-only fallback: remove any DSML-looking residue without executing
/// anything. Used when markup is malformed (so `parse_leaked_tool_calls`
/// returns `None`) yet the raw tags must never reach the transcript.
pub fn strip_dsml_residue(text: &str) -> String {
    let mut out = text.to_string();
    for dialect in DIALECTS {
        // Remove whole invoke blocks conservatively: from an invoke open to
        // its matching close when both exist. Unmatched fragments are
        // removed as plain markers below.
        loop {
            let Some(open) = out.find(dialect.invoke_open) else {
                break;
            };
            let Some(close_rel) = out[open..].find(dialect.invoke_close) else {
                break;
            };
            let close = open + close_rel + dialect.invoke_close.len();
            out.replace_range(open..close, "");
        }
        out = out.replace(dialect.calls_open, "");
        out = out.replace(dialect.calls_close, "");
        out = out.replace(dialect.invoke_close, "");
        // A dangling invoke open (truncated stream) up to end of line.
        while let Some(at) = out.find(dialect.invoke_open) {
            // Drop from the marker to the next `>` when present, else to end.
            let rest = &out[at..];
            if let Some(gt) = rest.find('>') {
                out.replace_range(at..at + gt + 1, "");
            } else {
                out.truncate(at);
                break;
            }
        }
        out = out.replace(dialect.marker, "");
    }
    // DeepSeek sentence framing tokens that occasionally leak alongside DSML.
    for token in [
        "<｜begin▁of▁sentence｜>",
        "<｜end▁of▁sentence｜>",
        "<｜User｜>",
        "<｜Assistant｜>",
        "<｜think｜>",
        "<|begin_of_sentence|>",
        "<|end_of_sentence|>",
        "<|User|>",
        "<|Assistant|>",
    ] {
        out = out.replace(token, "");
    }
    out
}

/// Clean visible assistant text: heal when possible, otherwise strip residue.
///
/// Returns `(cleaned_text, healed_calls)`. Never fails: malformed markup
/// yields stripped text with no calls rather than leaking raw tags.
pub fn sanitize_assistant_text(text: &str) -> (String, Vec<DsmlToolCall>) {
    if !contains_dsml(text) {
        return (sanitize_special_tokens(text), Vec::new());
    }
    if let Some((cleaned, calls)) = parse_leaked_tool_calls(text) {
        // A malformed sibling may survive alongside the healed call; make
        // sure no raw marker reaches the transcript.
        let cleaned = strip_dsml_residue(&cleaned);
        return (sanitize_special_tokens(&cleaned), calls);
    }
    (
        sanitize_special_tokens(&strip_dsml_residue(text)),
        Vec::new(),
    )
}

fn sanitize_special_tokens(text: &str) -> String {
    let mut out = text.to_string();
    for token in [
        "<｜begin▁of▁sentence｜>",
        "<｜end▁of▁sentence｜>",
        "<｜User｜>",
        "<｜Assistant｜>",
        "<|begin_of_sentence|>",
        "<|end_of_sentence|>",
    ] {
        out = out.replace(token, "");
    }
    out
}

/// Incremental DSML filter for streamed text content.
///
/// Feed content deltas through [`DsmlStreamFilter::push`]; it returns the text
/// that is safe to emit downstream. Text that could be (part of) a DSML marker
/// is withheld. Once a marker is confirmed the rest of the stream is buffered
/// and [`DsmlStreamFilter::finish`] returns the cleaned leftover text plus the
/// healed tool calls.
#[derive(Debug)]
pub struct DsmlStreamFilter {
    pending: String,
    in_dsml: bool,
    enabled: bool,
}

impl Default for DsmlStreamFilter {
    fn default() -> Self {
        Self::new(true)
    }
}

impl DsmlStreamFilter {
    pub fn new(enabled: bool) -> Self {
        Self {
            pending: String::new(),
            in_dsml: false,
            enabled,
        }
    }

    /// Append a content delta; returns the portion that is safe to emit now.
    pub fn push(&mut self, delta: &str) -> String {
        if !self.enabled {
            return delta.to_string();
        }
        self.pending.push_str(delta);
        if self.in_dsml {
            return String::new();
        }
        if let Some((_, start)) = detect_dialect(&self.pending) {
            self.in_dsml = true;
            let emit = self.pending[..start].to_string();
            self.pending.drain(..start);
            return emit;
        }
        // Withhold the longest tail that could still grow into the marker.
        let hold = longest_marker_prefix_suffix(&self.pending);
        let emit_len = self.pending.len() - hold;
        let emit = self.pending[..emit_len].to_string();
        self.pending.drain(..emit_len);
        emit
    }

    /// Consume the filter at end of stream. Returns any remaining visible text
    /// and the tool calls healed from buffered DSML markup.
    pub fn finish(self) -> (String, Vec<DsmlToolCall>) {
        if !self.in_dsml {
            return (self.pending, Vec::new());
        }
        match parse_leaked_tool_calls(&self.pending) {
            Some((cleaned, calls)) => {
                let cleaned = strip_dsml_residue(&cleaned);
                (cleaned, calls)
            }
            // Incomplete or unparseable markup: strip residue so raw tags
            // never reach the transcript, and heal nothing.
            None => (strip_dsml_residue(&self.pending), Vec::new()),
        }
    }
}

/// Length in bytes of the longest suffix of `text` that is a proper prefix of
/// any dialect's marker.
fn longest_marker_prefix_suffix(text: &str) -> usize {
    let mut best = 0;
    for dialect in DIALECTS {
        for (i, _) in dialect.marker.char_indices().skip(1) {
            if text.ends_with(&dialect.marker[..i]) {
                best = best.max(i);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENVELOPE: &str = "<｜DSML｜tool_calls>\n<｜DSML｜invoke name=\"shell\">\n<｜DSML｜parameter name=\"command\" string=\"true\">ls</｜DSML｜parameter>\n</｜DSML｜invoke>\n</｜DSML｜tool_calls>";

    #[test]
    fn parses_leaked_envelope() {
        let text = format!("read it.\n{ENVELOPE}");
        let (cleaned, calls) = parse_leaked_tool_calls(&text).expect("healed");
        assert_eq!(cleaned, "read it.\n");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
        let args: serde_json::Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["command"], "ls");
    }

    #[test]
    fn parses_ascii_fallback() {
        let text = "<|DSML|invoke name=\"shell\">\n<|DSML|parameter name=\"command\" string=\"true\">ls</|DSML|parameter>\n</|DSML|invoke>";
        let (cleaned, calls) = parse_leaked_tool_calls(text).expect("healed");
        assert_eq!(cleaned, "");
        assert_eq!(calls[0].name, "shell");
    }

    #[test]
    fn parses_double_bar_self_closing() {
        let text = "<｜｜DSML｜｜invoke name=\"exec_command\">\n<｜｜DSML｜｜invoke name=\"cmd\" string=\"echo alpha\" />\n</｜｜DSML｜｜invoke>";
        let (_, calls) = parse_leaked_tool_calls(text).expect("healed");
        let args: serde_json::Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(args["cmd"], "echo alpha");
    }

    #[test]
    fn plain_text_is_untouched() {
        assert!(parse_leaked_tool_calls("no markup here, just a < b").is_none());
    }

    #[test]
    fn malformed_calls_fail_closed() {
        let text = "<｜DSML｜invoke name=\"exec_command\"><｜DSML｜invoke name=\"cmd\" /></｜DSML｜invoke>";
        assert!(parse_leaked_tool_calls(text).is_none());
    }

    #[test]
    fn residue_strip_never_leaks_markers() {
        let text = "hi <｜DSML｜tool_calls>broken";
        let stripped = strip_dsml_residue(text);
        assert!(!stripped.contains("DSML"));
        assert!(stripped.contains("hi"));
    }

    #[test]
    fn sanitize_never_leaks() {
        let (cleaned, calls) = sanitize_assistant_text("hi <｜DSML｜invoke name=\"bad\">x");
        assert!(!cleaned.contains("DSML"));
        assert!(calls.is_empty());
    }

    #[test]
    fn stream_filter_holds_split_marker_and_heals() {
        let mut filter = DsmlStreamFilter::default();
        let mut emitted = String::new();
        emitted.push_str(&filter.push("look.\n<｜DS"));
        emitted.push_str(&filter.push("ML｜tool_calls>\n<｜DSML｜invoke name=\"shell\">\n<｜DSML｜parameter name=\"command\" string=\"true\">ls</｜DSML｜parameter>\n</｜DSML｜invoke>\n</｜DSML｜"));
        emitted.push_str(&filter.push("tool_calls>"));
        assert_eq!(emitted, "look.\n");
        let (leftover, calls) = filter.finish();
        assert_eq!(leftover, "");
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn stream_filter_passes_plain_text() {
        let mut filter = DsmlStreamFilter::default();
        assert_eq!(filter.push("hello "), "hello ");
        let (leftover, calls) = filter.finish();
        assert_eq!(leftover, "");
        assert!(calls.is_empty());
    }
}
