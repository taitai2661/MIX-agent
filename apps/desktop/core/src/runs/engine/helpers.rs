use super::*;

pub(super) const MAX_SNAPSHOT_FILES: usize = 128;

/// How many Runs keep their undo history in memory.
pub(super) const MAX_SNAPSHOT_RUNS: usize = 24;

/// Resolve a Tool's path argument to an absolute key for the read fingerprint
/// map. `std::path::absolute` folds `.` and makes relative/absolute spellings
/// of the same file compare equal.
///
/// Returns `None` for paths outside the workspace: those are not snapshotted
/// or fingerprinted, so a crafted path cannot turn Undo into an arbitrary
/// read/write primitive.
pub(super) fn normalize_tool_path(
    root: Option<&std::path::Path>,
    path: &str,
) -> Option<std::path::PathBuf> {
    let joined = resolve_snapshot_path(root, path)?;
    Some(std::path::absolute(&joined).unwrap_or(joined))
}

/// Resolve a Tool's path argument against the workspace root.
///
/// Unlike the old implementation this does not return absolute paths verbatim
/// and does not leave `..` components for the OS to resolve. It reuses the
/// Files tool's resolver, which normalizes the path, follows in-workspace
/// symlinks only, and rejects anything that escapes the root. Only regular
/// files are accepted: `/dev/zero`, FIFOs and directories would otherwise make
/// the pre-run snapshot read hang or exhaust memory.
pub(super) fn resolve_snapshot_path(
    root: Option<&std::path::Path>,
    path: &str,
) -> Option<std::path::PathBuf> {
    let root = root?.canonicalize().ok()?;
    let (resolved, writable) =
        mixd_tools::files::FilesTool::resolve_target_in(&root, &[], path).ok()?;
    if !writable {
        return None;
    }
    match std::fs::metadata(&resolved) {
        Ok(meta) if !meta.is_file() => None,
        Ok(_) => Some(resolved),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Some(resolved),
        Err(_) => None,
    }
}

/// How many read-only Tool calls may be in flight at once.
///
/// Bounded rather than unbounded: a model that emits twenty greps should not
/// spawn twenty directory walks that compete for the same disk.
pub(super) const MAX_CONCURRENT_READS: usize = 4;

/// Whether a call is safe to dispatch alongside another one in the same turn.
///
/// Deliberately narrower than [`crate::runs::stagnation::is_read_only`]: network
/// fetches are read-only in the "does not mutate the workspace" sense, but
/// bursting them at a provider or a site invites rate limits, so they keep the
/// serial path. These are all local, non-sensitive reads.
pub(super) fn can_run_concurrently(name: &str, args: &serde_json::Value) -> bool {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
    match name {
        "files" => matches!(action, "read" | "list"),
        "glob" | "grep" | "symbols" | "lsp" | "skill" | "skill_resource" => true,
        "git" => matches!(action, "status" | "diff" | "log" | "show" | "blame"),
        "knowledge" => action == "search",
        _ => false,
    }
}

/// Trim a tool transcript before putting it in a Run event payload. Failure
/// output can be hundreds of kilobytes; the UI only needs enough to see why.
pub(super) fn truncate_event_text(text: &str) -> String {
    const MAX: usize = 4_000;
    if text.len() <= MAX {
        return text.to_string();
    }
    let cut = text
        .char_indices()
        .take_while(|(index, _)| *index < MAX)
        .last()
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    format!("{}…", &text[..cut])
}

/// Remove a leading `system` head block injected by the compression step.
pub(super) fn strip_head(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .skip_while(|m| {
            m.role == "system"
                && (m.content.starts_with("<task_state>")
                    || m.content.starts_with("<compressed_history>"))
        })
        .cloned()
        .collect()
}

/// Ask the Provider to compress older turns. Returns `None` when the Provider
/// cannot be reached so the caller keeps the original history.
pub(super) async fn summarize_turns(
    provider: &dyn Provider,
    model: &str,
    older: &[Message],
) -> Option<String> {
    let mut transcript = String::new();
    for m in older {
        transcript.push_str(&format!("{}: {}\n", m.role, m.content));
    }
    let req = ProviderRequest {
        model: model.to_string(),
        messages: vec![Message::text(
            "user",
            format!(
                "以下の会話を、後続ターンで参照できる高密度な要約に圧縮してください。\
                 重要な事実・決定・未完了の作業・ファイルパスを必ず残してください。\n\n{}",
                transcript
            ),
        )],
        system: Some("You compress conversation history without losing actionable detail.".into()),
        tools: Arc::new(Vec::new()),
        temperature: Some(SUMMARIZER_TEMPERATURE),
        top_p: None,
        max_tokens: Some(SUMMARIZER_MAX_TOKENS),
        reasoning: None,
        stream: true,
        session_id: None,
        context_window: None,
        keep_alive: None,
    };
    let mut stream = match provider.stream(req).await {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(?err, "summarize_turns: provider stream failed");
            return None;
        }
    };
    let mut out = String::new();
    while let Some(ev) = futures_util::StreamExt::next(&mut stream).await {
        match ev {
            StreamEvent::TextDelta(delta) => out.push_str(&delta),
            StreamEvent::Finished { .. } => break,
            StreamEvent::Error(e) => {
                tracing::warn!(?e, "summarize_turns: stream error");
                return None;
            }
            _ => {}
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(super) fn history_to_message(h: &HistoryMessage) -> Message {
    Message {
        role: h.role.clone(),
        content: h.content.clone(),
        tool_calls: h.tool_calls.clone(),
        tool_call_id: h.tool_call_id.clone(),
        name: h.name.clone(),
        is_error: h.is_error,
        native: h.native.clone(),
    }
}

/// Render the block that is prepended to every Provider request: the goal the
/// user stated plus whatever checklist the agent has built with the `todo` tool.
///
/// Two separate concerns, one block, so the adapter can fold it into whichever
/// system channel it uses.
pub(super) async fn render_head(
    task_state: &crate::context::TaskState,
    todos: &mixd_tools::todo::TodoStore,
    run_id: RunId,
) -> String {
    let mut head = task_state.render();
    let checklist = todos.render(run_id).await;
    if !checklist.is_empty() {
        head.push('\n');
        head.push_str(&checklist);
        head.push('\n');
    }
    head
}

/// Per-turn ceiling for persisted Responses items. Encrypted reasoning is
/// usually a few KiB; a runaway blob should not bloat the transcript or a
/// checkpoint, and dropping it only costs reasoning continuity, not the turn.
const NATIVE_HISTORY_MAX_BYTES: usize = 256 * 1024;

pub(super) fn message_to_history(m: &Message) -> HistoryMessage {
    let native = if m.native.is_empty() {
        Vec::new()
    } else {
        match serde_json::to_vec(&m.native) {
            Ok(encoded) if encoded.len() <= NATIVE_HISTORY_MAX_BYTES => m.native.clone(),
            Ok(encoded) => {
                tracing::warn!(
                    bytes = encoded.len(),
                    limit = NATIVE_HISTORY_MAX_BYTES,
                    "dropping oversized native provider items from the transcript"
                );
                Vec::new()
            }
            Err(err) => {
                tracing::warn!(%err, "could not encode native provider items");
                Vec::new()
            }
        }
    };
    HistoryMessage {
        role: m.role.clone(),
        content: m.content.clone(),
        tool_call_id: m.tool_call_id.clone(),
        name: m.name.clone(),
        tool_calls: m.tool_calls.clone(),
        is_error: m.is_error,
        native,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mixd-snapshot-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn snapshot_paths_stay_inside_the_workspace() {
        let root = temp_root();
        assert!(resolve_snapshot_path(Some(&root), "notes.txt").is_some());
        assert!(resolve_snapshot_path(Some(&root), "sub/notes.txt").is_some());
        // Traversal, absolute paths outside the root, and character devices
        // are all refused instead of being read/restored verbatim.
        assert!(resolve_snapshot_path(Some(&root), "../escape.txt").is_none());
        // `/etc/passwd` is not an absolute path on Windows, so use a real
        // drive-qualified path there instead of asserting Unix semantics.
        #[cfg(unix)]
        {
            assert!(resolve_snapshot_path(Some(&root), "/etc/passwd").is_none());
            assert!(resolve_snapshot_path(Some(&root), "/dev/zero").is_none());
        }
        #[cfg(windows)]
        assert!(resolve_snapshot_path(Some(&root), r"C:\Windows\win.ini").is_none());
        assert!(resolve_snapshot_path(None, "notes.txt").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_is_not_snapshotted_as_a_file() {
        let root = temp_root();
        assert!(resolve_snapshot_path(Some(&root), ".").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
