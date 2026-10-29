use super::*;

pub(super) const MAX_SNAPSHOT_FILES: usize = 128;

/// How many Runs keep their undo history in memory.
pub(super) const MAX_SNAPSHOT_RUNS: usize = 24;

/// Resolve a Tool's path argument to an absolute key for the read fingerprint
/// map. `std::path::absolute` folds `.` and makes relative/absolute spellings
/// of the same file compare equal.
pub(super) fn normalize_tool_path(
    root: Option<&std::path::Path>,
    path: &str,
) -> std::path::PathBuf {
    let joined = resolve_snapshot_path(root, path);
    std::path::absolute(&joined).unwrap_or(joined)
}

/// Resolve a Tool's path argument against the workspace root.
pub(super) fn resolve_snapshot_path(
    root: Option<&std::path::Path>,
    path: &str,
) -> std::path::PathBuf {
    let candidate = std::path::Path::new(path);
    if candidate.is_absolute() {
        return candidate.to_path_buf();
    }
    match root {
        Some(root) => root.join(candidate),
        None => candidate.to_path_buf(),
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
        "glob" | "grep" | "symbols" => true,
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

pub(super) fn message_to_history(m: &Message) -> HistoryMessage {
    HistoryMessage {
        role: m.role.clone(),
        content: m.content.clone(),
        tool_call_id: m.tool_call_id.clone(),
        name: m.name.clone(),
        tool_calls: m.tool_calls.clone(),
        is_error: m.is_error,
    }
}
