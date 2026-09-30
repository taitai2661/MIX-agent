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
/// Compression is attempted once the estimated prompt exceeds this many chars.
pub const COMPRESS_THRESHOLD_CHARS: usize = 120_000;

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
    /// `true` when nothing was compressed because the prompt was short enough.
    pub skipped: bool,
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
    mut messages: Vec<crate::providers_bridge::Message>,
    mut summary: TieredSummary,
    summarize: F,
) -> CompressionOutcome
where
    F: FnOnce(Vec<crate::providers_bridge::Message>) -> Fut,
    Fut: std::future::Future<Output = Option<String>>,
{
    if estimate_chars(&messages) < COMPRESS_THRESHOLD_CHARS {
        return CompressionOutcome {
            messages,
            summary,
            skipped: true,
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
            }
        }
        // Summariser failed: keep everything rather than dropping context.
        _ => CompressionOutcome {
            messages,
            summary,
            skipped: true,
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
