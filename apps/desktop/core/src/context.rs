//! Hierarchical context summarisation.
//!
//! Mirrors the Docker edition's `context.tiered_summary` behaviour:
//!
//! - An **active summary** (bounded to [`ACTIVE_SUMMARY_CHAR_BUDGET`]) that
//!   compresses older conversation turns into one block.
//! - Up to [`ARCHIVE_ROW_LIMIT`] **archive rows** holding the most recent
//!   compression results.
//! - `task_state` (goal / plan / pending / important_facts / artifacts /
//!   open_questions) is kept in a head block that is never truncated, so the
//!   agent always knows what it is doing.
//! - The current turn's Tool call / Tool result pairs and signed blocks are
//!   never cut, even when a summary is produced in the same step.

use serde::{Deserialize, Serialize};

/// Hard cap on the active summary so a pathological summary cannot blow the
/// context window on its own.
pub const ACTIVE_SUMMARY_CHAR_BUDGET: usize = 4000;
/// Number of archived summaries to keep.
pub const ARCHIVE_ROW_LIMIT: usize = 12;
/// Compression threshold used when the model's context window is unknown.
pub const COMPRESS_THRESHOLD_CHARS: usize = 120_000;

/// Rough characters per token. English prose and source code both land near
/// four, and a heuristic that is 20% off only moves the compression point by
/// 20% — it does not need to be a tokenizer.
pub const CHARS_PER_TOKEN: usize = 4;

/// Share of the context window the prompt may occupy before compressing.
///
/// The remainder is headroom for the model's own output and for the tool
/// schemas, which are re-sent on every turn and are not part of `messages`.
const PROMPT_BUDGET_PERCENT: usize = 60;

/// Floor for the derived threshold, so a small local model does not compress on
/// every single turn — compression itself costs a Provider call.
const MIN_COMPRESS_THRESHOLD_CHARS: usize = 16_000;

/// Total characters of Tool results kept inline before the oldest ones are
/// replaced with a pointer.
pub const TOOL_RESULT_CHAR_BUDGET: usize = 60_000;

/// Characters the prompt may reach before older turns are summarised.
///
/// `window_tokens == 0` means "unknown" and keeps the fixed threshold. The
/// previous behaviour was this fixed threshold for every model, which was both
/// late for an 8k local model — the prompt was already over the window before
/// anything was compressed — and needlessly eager for a 200k one.
pub fn compress_threshold_chars(window_tokens: u32) -> usize {
    if window_tokens == 0 {
        return COMPRESS_THRESHOLD_CHARS;
    }
    let budget = window_tokens as usize * CHARS_PER_TOKEN * PROMPT_BUDGET_PERCENT / 100;
    budget.max(MIN_COMPRESS_THRESHOLD_CHARS)
}

/// Replace the oldest oversized Tool results with a pointer.
///
/// The message itself is never removed: a `tool` message is only valid when the
/// assistant turn that requested it is still in the list, and dropping it
/// produces a sequence strict providers reject. Replacing the body keeps the
/// pairing intact while giving the context back. Returns the rewritten list and
/// how many results were elided, so the UI can say so.
pub fn evict_tool_results(
    mut messages: Vec<crate::providers_bridge::Message>,
    budget: usize,
) -> (Vec<crate::providers_bridge::Message>, usize) {
    let total: usize = messages
        .iter()
        .filter(|m| m.role == "tool")
        .map(|m| m.content.chars().count())
        .sum();
    if total <= budget {
        return (messages, 0);
    }
    let mut remaining = total;
    let mut evicted = 0usize;
    for message in messages.iter_mut() {
        if remaining <= budget {
            break;
        }
        if message.role != "tool" {
            continue;
        }
        let len = message.content.chars().count();
        let placeholder =
            format!("[tool result elided: {len} chars, re-run the call if you still need it]");
        let placeholder_len = placeholder.chars().count();
        if placeholder_len >= len {
            continue;
        }
        message.content = placeholder;
        remaining = remaining - len + placeholder_len;
        evicted += 1;
    }
    (messages, evicted)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TaskState {
    pub goal: String,
    pub plan: Vec<String>,
    pub pending: Vec<String>,
    pub important_facts: Vec<String>,
    pub artifacts: Vec<String>,
    pub open_questions: Vec<String>,
}

impl TaskState {
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("<task_state>\n");
        if !self.goal.is_empty() {
            out.push_str(&format!("goal: {}\n", self.goal));
        }
        for (label, items) in [
            ("plan", &self.plan),
            ("pending", &self.pending),
            ("important_facts", &self.important_facts),
            ("artifacts", &self.artifacts),
            ("open_questions", &self.open_questions),
        ] {
            if items.is_empty() {
                continue;
            }
            out.push_str(&format!("{}:\n", label));
            for item in items {
                out.push_str(&format!("  - {}\n", item));
            }
        }
        out.push_str("</task_state>\n");
        out
    }

    pub fn is_empty(&self) -> bool {
        self.goal.is_empty()
            && self.plan.is_empty()
            && self.pending.is_empty()
            && self.important_facts.is_empty()
            && self.artifacts.is_empty()
            && self.open_questions.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TieredSummary {
    pub active: String,
    pub archive: Vec<String>,
}

impl TieredSummary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold `text` into the active summary, pushing the previous active row
    /// into the archive when the character budget would be exceeded.
    pub fn absorb(&mut self, text: &str) {
        let mut candidate = String::new();
        if !self.active.is_empty() {
            candidate.push_str(&self.active);
            candidate.push_str("\n\n");
        }
        candidate.push_str(text.trim());
        if candidate.chars().count() > ACTIVE_SUMMARY_CHAR_BUDGET {
            if !self.active.is_empty() {
                self.archive.push(std::mem::take(&mut self.active));
            }
            while self.archive.len() > ARCHIVE_ROW_LIMIT {
                self.archive.remove(0);
            }
            // Trim the new text to fit on its own.
            self.active = text
                .trim()
                .chars()
                .take(ACTIVE_SUMMARY_CHAR_BUDGET)
                .collect();
        } else {
            self.active = candidate;
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for row in &self.archive {
            out.push_str(row);
            out.push_str("\n\n");
        }
        out.push_str(&self.active);
        out
    }

    pub fn char_count(&self) -> usize {
        self.render().chars().count()
    }
}

/// The result of a compression pass.
#[derive(Debug, Clone)]
pub struct CompressionOutcome {
    /// The rebuilt message list.
    pub messages: Vec<crate::providers_bridge::Message>,
    pub summary: TieredSummary,
    /// `true` when no summarisation happened because the prompt was short
    /// enough. Tool results may still have been elided — that is reported
    /// separately.
    pub skipped: bool,
    /// How many oversized Tool results were replaced with a pointer.
    pub evicted_tool_results: usize,
}

/// Estimate the character cost of a message list.
pub fn estimate_chars(messages: &[crate::providers_bridge::Message]) -> usize {
    messages
        .iter()
        .map(|m| m.content.chars().count() + m.role.chars().count() + 16)
        .sum()
}

/// A `tool` message is a Tool *result*: it is only meaningful when the
/// assistant turn that requested it is still in the list. Splitting between the
/// two produces a sequence strict providers reject.
fn is_tool_result(m: &crate::providers_bridge::Message) -> bool {
    m.role == "tool"
}

/// Pick a split index that keeps every assistant/tool pair intact.
///
/// Starts from the "keep the last third" estimate and walks forward until the
/// first retained message is not an orphaned Tool result. This can only make the
/// retained tail larger, never drop a Tool result.
fn safe_split(messages: &[crate::providers_bridge::Message]) -> usize {
    let keep = (messages.len() / 3).max(4);
    let mut split = messages.len().saturating_sub(keep);
    while split < messages.len() && is_tool_result(&messages[split]) {
        split += 1;
    }
    // If the whole tail is Tool results, keep the entire list rather than emit an
    // invalid sequence.
    if split >= messages.len() {
        messages.len()
    } else {
        split
    }
}

/// Compress the oldest eligible turns into the active summary.
///
/// `summarize` receives exactly the slice that is about to be dropped, so the
/// summary covers the history being removed rather than re-summarising the tail
/// that is kept verbatim. When it returns `None` the engine keeps the original
/// history so no context is silently lost.
///
/// The history is taken by value and handed back unchanged whenever nothing was
/// compressed: the agent loop rebuilds the outgoing request every step, and
/// copying the whole conversation just to find out it is still needed was the
/// single largest per-step cost in the loop.
pub async fn compress<F, Fut>(
    messages: Vec<crate::providers_bridge::Message>,
    summary: TieredSummary,
    summarize: F,
) -> CompressionOutcome
where
    F: FnOnce(Vec<crate::providers_bridge::Message>) -> Fut,
    Fut: std::future::Future<Output = Option<String>>,
{
    compress_with_budget(messages, summary, 0, summarize).await
}

/// [`compress`] with the selected model's context window.
///
/// The window is what makes the threshold meaningful; the old fixed count was
/// the same for every model.
pub async fn compress_with_budget<F, Fut>(
    messages: Vec<crate::providers_bridge::Message>,
    mut summary: TieredSummary,
    window_tokens: u32,
    summarize: F,
) -> CompressionOutcome
where
    F: FnOnce(Vec<crate::providers_bridge::Message>) -> Fut,
    Fut: std::future::Future<Output = Option<String>>,
{
    // Shrink oversized Tool results first: one 200 KB `terminal` dump can carry
    // the prompt over the threshold by itself, and summarising the whole
    // conversation is a far more expensive answer to that than dropping the
    // dump.
    let (mut messages, evicted_tool_results) =
        evict_tool_results(messages, TOOL_RESULT_CHAR_BUDGET);

    if estimate_chars(&messages) < compress_threshold_chars(window_tokens) {
        return CompressionOutcome {
            messages,
            summary,
            skipped: true,
            evicted_tool_results,
        };
    }

    // Keep the last third of turns verbatim, without ever orphaning a Tool
    // result or splitting an assistant tool_call from its results.
    let split = safe_split(&messages);
    if split == 0 {
        return CompressionOutcome {
            messages,
            summary,
            skipped: true,
            evicted_tool_results,
        };
    }
    let older = messages[..split].to_vec();

    match summarize(older).await {
        Some(text) if !text.trim().is_empty() => {
            summary.absorb(&text);
            let mut rebuilt = Vec::new();
            if !summary.active.is_empty() {
                rebuilt.push(crate::providers_bridge::Message {
                    role: "system".into(),
                    content: format!(
                        "<compressed_history>\n{}\n</compressed_history>",
                        summary.active
                    ),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    name: None,
                    is_error: false,
                });
            }
            // `split_off` moves the retained tail instead of copying it.
            rebuilt.extend(messages.split_off(split));
            CompressionOutcome {
                messages: rebuilt,
                summary,
                skipped: false,
                evicted_tool_results,
            }
        }
        // Summariser failed: keep everything rather than dropping context.
        _ => CompressionOutcome {
            messages,
            summary,
            skipped: true,
            evicted_tool_results,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers_bridge::Message;

    fn msg(role: &str, content: &str) -> Message {
        Message::text(role, content)
    }

    #[test]
    fn an_unknown_window_keeps_the_fixed_threshold() {
        assert_eq!(compress_threshold_chars(0), COMPRESS_THRESHOLD_CHARS);
    }

    #[test]
    fn a_small_window_compresses_much_earlier_than_a_large_one() {
        let small = compress_threshold_chars(8_000);
        let large = compress_threshold_chars(200_000);
        assert!(small < large);
        assert_eq!(large, 200_000 * CHARS_PER_TOKEN * 60 / 100);
        assert!(small >= MIN_COMPRESS_THRESHOLD_CHARS);
    }

    #[test]
    fn an_absurdly_small_window_still_has_a_floor() {
        assert_eq!(compress_threshold_chars(100), MIN_COMPRESS_THRESHOLD_CHARS);
    }

    #[test]
    fn oversized_tool_results_are_elided_from_the_oldest_end() {
        let mut messages = vec![msg("user", "go")];
        for i in 0..3 {
            messages.push(msg("assistant", &format!("call {i}")));
            messages.push(msg("tool", &"x".repeat(40_000)));
        }
        let (out, evicted) = evict_tool_results(messages, 50_000);
        assert!(evicted >= 1, "the budget must force at least one eviction");
        assert_eq!(out.len(), 7, "no message may be dropped, only rewritten");
        // The oldest result lost its body; the newest kept it.
        assert!(out[2].content.starts_with("[tool result elided"));
        assert_eq!(out[6].content.chars().count(), 40_000);
        // The pairing the providers require is untouched.
        for i in 0..7 {
            if out[i].role == "tool" {
                assert_eq!(out[i - 1].role, "assistant");
            }
        }
    }

    #[test]
    fn a_prompt_within_budget_is_left_alone() {
        let messages = vec![msg("user", "go"), msg("tool", "small")];
        let (out, evicted) = evict_tool_results(messages, TOOL_RESULT_CHAR_BUDGET);
        assert_eq!(evicted, 0);
        assert_eq!(out[1].content, "small");
    }

    #[test]
    fn absorb_respects_budget_and_archives() {
        let mut s = TieredSummary::new();
        s.absorb(&"a".repeat(ACTIVE_SUMMARY_CHAR_BUDGET - 10));
        s.absorb(&"b".repeat(ACTIVE_SUMMARY_CHAR_BUDGET - 10));
        assert_eq!(s.archive.len(), 1);
        assert!(s.char_count() <= ACTIVE_SUMMARY_CHAR_BUDGET * 2);
    }

    #[tokio::test]
    async fn compress_skips_short_prompts() {
        let messages = vec![msg("user", "hello"), msg("assistant", "hi")];
        let out = compress(messages, TieredSummary::new(), |_| async { None }).await;
        assert!(out.skipped);
        assert_eq!(out.messages.len(), 2);
    }

    #[tokio::test]
    async fn compress_keeps_tail_verbatim() {
        let big = "x".repeat(COMPRESS_THRESHOLD_CHARS);
        let mut messages = vec![msg("user", &big)];
        for i in 0..10 {
            messages.push(msg("assistant", &format!("turn {}", i)));
        }
        messages.push(msg("tool", "signed tool result"));
        let out = compress(messages, TieredSummary::new(), |_| async {
            Some("earlier turns summarised".to_string())
        })
        .await;
        assert!(!out.skipped);
        assert_eq!(out.summary.active, "earlier turns summarised");
        // The last turn (tool result) is still present verbatim.
        assert_eq!(out.messages.last().unwrap().content, "signed tool result");
    }

    #[tokio::test]
    async fn compress_fails_open_when_summariser_fails() {
        let big = "x".repeat(COMPRESS_THRESHOLD_CHARS);
        let mut messages = vec![msg("user", &big)];
        for i in 0..10 {
            messages.push(msg("assistant", &format!("turn {}", i)));
        }
        let before = messages.len();
        let out = compress(messages, TieredSummary::new(), |_| async { None }).await;
        assert!(out.skipped);
        assert_eq!(out.messages.len(), before);
    }

    /// A `tool` message is a Tool *result*: it is only valid when the assistant
    /// turn that requested it is still present. The naive "keep the last third"
    /// boundary can land on one, which strict providers reject outright.
    #[tokio::test]
    async fn compress_never_orphans_a_tool_result() {
        let big = "x".repeat(COMPRESS_THRESHOLD_CHARS);
        // 13 messages => keep = 13/3 = 4, so the naive boundary is index 9.
        // Place the Tool result exactly there, with its requester just before.
        let mut messages = vec![msg("user", &big)];
        for i in 0..7 {
            messages.push(msg("assistant", &format!("filler {}", i)));
        }
        messages.push(msg("assistant", "calling a tool"));
        messages.push(msg("tool", "tool result payload"));
        for i in 0..3 {
            messages.push(msg("assistant", &format!("tail {}", i)));
        }
        assert_eq!(messages.len(), 13);
        assert_eq!(
            messages[9].role, "tool",
            "the naive split starts on this orphan"
        );

        let out = compress(messages, TieredSummary::new(), |_| async {
            Some("summary".to_string())
        })
        .await;
        assert!(!out.skipped);

        // The retained history must never open with a Tool result.
        let retained: Vec<&crate::providers_bridge::Message> =
            out.messages.iter().skip(1).collect();
        assert!(
            !retained.is_empty(),
            "compression dropped the entire conversation"
        );
        assert_ne!(
            retained[0].role, "tool",
            "retained history opens with an orphaned Tool result; strict \
             providers (OpenAI/Anthropic) reject this sequence"
        );
        // And no Tool result may appear without its requester.
        for (i, m) in retained.iter().enumerate() {
            if m.role == "tool" {
                assert!(
                    i > 0 && retained[i - 1].role == "assistant",
                    "Tool result at {i} is not preceded by its assistant request"
                );
            }
        }
    }

    /// The summariser must only see the slice that is actually removed;
    /// summarising the whole list would duplicate the verbatim tail in the
    /// prompt and waste tokens.
    #[tokio::test]
    async fn compress_summarises_only_the_dropped_slice() {
        let big = "x".repeat(COMPRESS_THRESHOLD_CHARS);
        let mut messages = vec![msg("user", &big)];
        for i in 0..10 {
            messages.push(msg("assistant", &format!("turn {}", i)));
        }
        let total = messages.len();

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let out = compress(messages.clone(), TieredSummary::new(), move |older| {
            let sink = sink.clone();
            async move {
                *sink.lock().expect("lock") = older.iter().map(|m| m.content.clone()).collect();
                Some("summary".to_string())
            }
        })
        .await;
        assert!(!out.skipped);

        let summarised = seen.lock().expect("lock").clone();
        assert!(
            summarised.len() < total,
            "summariser saw the whole list ({}) instead of the dropped prefix",
            summarised.len()
        );
        // Everything handed over is a prefix of the original list.
        assert_eq!(
            summarised,
            messages[..summarised.len()]
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
        );
    }
}
