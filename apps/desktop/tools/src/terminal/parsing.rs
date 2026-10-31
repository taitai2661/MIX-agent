use super::*;

pub(crate) fn find_line(text: &str, needle: &str) -> Option<usize> {
    text.lines().enumerate().find_map(|(index, line)| {
        let normalized = strip_terminal_escape_sequences(line);
        let normalized = normalized.trim_end_matches('\r');
        (normalized == needle || normalized.ends_with(needle)).then_some(index)
    })
}

/// Strip ANSI CSI/OSC escape sequences when matching protocol markers. The raw
/// transcript remains unchanged so command output is not silently reformatted.
pub(crate) fn strip_terminal_escape_sequences(text: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Text,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }

    let mut state = State::Text;
    let mut clean = String::with_capacity(text.len());
    for ch in text.chars() {
        state = match state {
            State::Text if ch == '\u{1b}' => State::Escape,
            State::Text => {
                clean.push(ch);
                State::Text
            }
            State::Escape => match ch {
                '[' => State::Csi,
                ']' => State::Osc,
                _ => State::Text,
            },
            State::Csi if ('@'..='~').contains(&ch) => State::Text,
            State::Csi => State::Csi,
            State::Osc if ch == '\u{7}' => State::Text,
            State::Osc if ch == '\u{1b}' => State::OscEscape,
            State::Osc => State::Osc,
            State::OscEscape if ch == '\\' => State::Text,
            State::OscEscape if ch == '\u{1b}' => State::OscEscape,
            State::OscEscape => State::Osc,
        };
    }
    clean
}

/// Slice real command output out of a raw PTY transcript: everything strictly
/// between the start and done markers. The shell echoes the whole injected
/// script (`__mixd_rc=$?`, the `printf` lines, …), which lives outside that
/// window and must never leak into the tool result.
pub(crate) fn slice_output(text: &str, start_marker: &str, sentinel: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let marker_at = find_line(text, sentinel).unwrap_or(lines.len());
    let begin = match find_line(text, start_marker) {
        // The marker itself is protocol, not output.
        Some(start_at) => (start_at + 1).min(marker_at).min(lines.len()),
        // No start marker (timeout residue, a killed shell): keep everything
        // up to the done marker rather than dropping the first line.
        None => 0,
    };
    let end = marker_at.min(lines.len());
    lines[begin..end].join("\n")
}

/// Cap the returned output, saying how much was dropped.
///
/// Shared with the workspace `git_diff` preview path via
/// [`truncate_to_char_boundary`]: both caps must cut on a character boundary
/// (the release profile is `panic = "abort"`), so the boundary walk lives in
/// one place. See `app/src/commands/workspace.rs` (`git_diff` truncation).
pub(crate) fn truncate_output(text: &str) -> String {
    if text.len() <= MAX_OUTPUT_BYTES {
        return text.to_string();
    }
    let head = truncate_to_char_boundary(text, MAX_OUTPUT_BYTES);
    let cut = head.len();
    let kept_lines = head.lines().count();
    format!(
        "{head}\n… [truncated: {cut} of {} bytes shown, {kept_lines} lines]",
        text.len()
    )
}

/// Read the shell's exit-status file.
///
/// The protocol pre-creates this file and the shell overwrites it with a few
/// ASCII digits. PowerShell's `File.WriteAllText` has historically produced
/// UTF-8 with a BOM (and `Write-Output`/`Set-Content` can produce UTF-16), so
/// decoding accepts the common BOM variants instead of silently reporting
/// `exit_code: null` on Windows.
pub(crate) fn read_exit_code(path: &std::path::Path) -> Option<i32> {
    let raw = std::fs::read(path).ok()?;
    let text = decode_status_text(&raw);
    // Strict validation: a few ASCII digits (plus an optional leading `-`),
    // nothing else. A bloated or non-numeric file means something other than
    // our shell wrapper wrote it.
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 11 {
        return None;
    }
    let digits = trimmed.strip_prefix('-').unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    trimmed.parse::<i32>().ok()
}

/// Decode UTF-8 / UTF-8-with-BOM / UTF-16 LE/BE status files.
pub(crate) fn decode_status_text(raw: &[u8]) -> String {
    if let Some(rest) = raw.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = raw.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, u16::from_be_bytes);
    }
    // BOM-less UTF-16LE is easy to spot for a short digit string: every
    // second byte is NUL.
    if raw.len() >= 2
        && raw.len().is_multiple_of(2)
        && raw.iter().skip(1).step_by(2).all(|byte| *byte == 0)
    {
        return decode_utf16(raw, u16::from_le_bytes);
    }
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    String::from_utf8_lossy(raw).into_owned()
}

pub(crate) fn decode_utf16(rest: &[u8], decode: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = rest
        .chunks_exact(2)
        .map(|chunk| decode([chunk[0], chunk[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Keep the end of a transcript for timeout diagnostics.
///
/// The start of a PTY transcript is startup noise and the marker lines that
/// were already parsed; the useful part when a marker never arrives is the
/// tail. Count in characters so a multi-byte UTF-8 prompt cannot panic.
pub(crate) fn diagnostic_tail(text: &str, max_chars: usize) -> String {
    let mut chars: Vec<char> = text.chars().rev().take(max_chars).collect();
    chars.reverse();
    chars.into_iter().collect()
}
