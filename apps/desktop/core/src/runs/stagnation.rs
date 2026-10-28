//! Stagnation detection.
//!
//! An agent that is going nowhere usually looks like one of three things: the
//! same tool call failing over and over, the same file being re-read without
//! acting on it, or a provider that keeps erroring. Each produces a repair hint
//! that the engine injects as a `user`-role turn.
//!
//! The classifier has to be fed *facts*, not guesses. An earlier version mapped
//! every call to a single tool named `files` into `RepeatedRead`, so a
//! `files.write` counted as a read and the read-loop branch fired on healthy
//! work; `ToolFailure` and `ProviderError` were never emitted at all, which left
//! two of the three branches dead. [`classify_tool_call`] fixes that by looking
//! at what the call actually does.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// How many recent signals each rule judges.
pub const WINDOW: usize = 12;

/// Smallest window any rule needs before it may fire.
///
/// The failure and read-loop rules only need half a window to be confident.
/// Gating all of them on a *full* window meant a run that failed six times and
/// gave up never got a repair hint at all — the common case was the one case
/// the detector stayed silent about.
pub const MIN_SIGNALS: usize = WINDOW / 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StagnationKind {
    RepeatedFailure,
    ReadLoop,
    ProviderUnstable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagnationReport {
    pub kind: StagnationKind,
    pub hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnSignal {
    /// The model produced a normal answer.
    Progress,
    /// The model issued a Tool call.
    ToolCall,
    /// The model changed something on disk or in a shell.
    ///
    /// Kept separate from [`TurnSignal::ToolCall`] because a long run of pure
    /// calls that never change anything is a stall, while a long run of edits
    /// is a refactor in progress. Collapsing the two made every healthy
    /// multi-file change look like a hang.
    Mutation,
    /// A Tool call failed.
    ToolFailure,
    /// The model read the same resource repeatedly.
    RepeatedRead,
    /// The provider returned an error.
    ProviderError,
}

pub struct StagnationDetector {
    window: Vec<TurnSignal>,
    /// Resource keys the run has read, so a second read of the same thing is
    /// distinguishable from ordinary exploration.
    seen_reads: HashSet<String>,
}

impl StagnationDetector {
    pub fn new() -> Self {
        Self {
            window: Vec::new(),
            seen_reads: HashSet::new(),
        }
    }

    /// Note a read of `key` without a full signal list. Returns `true` when this
    /// is a repeat of something already read in this run.
    pub fn note_read(&mut self, key: impl Into<String>) -> bool {
        !self.seen_reads.insert(key.into())
    }

    pub fn observe(
        &mut self,
        signals: impl IntoIterator<Item = TurnSignal>,
    ) -> Option<StagnationReport> {
        self.window.extend(signals);
        if self.window.len() > WINDOW * 2 {
            let excess = self.window.len() - WINDOW * 2;
            self.window.drain(..excess);
        }
        if self.window.len() < MIN_SIGNALS {
            return None;
        }
        let span = self.window.len().min(WINDOW);
        let recent = &self.window[self.window.len() - span..];
        let count = |wanted: TurnSignal| recent.iter().filter(|s| **s == wanted).count();

        let failures = count(TurnSignal::ToolFailure);
        let provider_errors = count(TurnSignal::ProviderError);
        let repeated_reads = count(TurnSignal::RepeatedRead);
        let progress = count(TurnSignal::Progress) + count(TurnSignal::Mutation);

        if failures >= MIN_SIGNALS {
            return Some(StagnationReport {
                kind: StagnationKind::RepeatedFailure,
                hint:
                    "the same operation keeps failing; read the error text and change the approach"
                        .to_string(),
            });
        }
        if provider_errors >= 3 {
            return Some(StagnationReport {
                kind: StagnationKind::ProviderUnstable,
                hint: "the provider is unstable; retry once, then fall back to a smaller request"
                    .to_string(),
            });
        }
        if repeated_reads >= MIN_SIGNALS {
            return Some(StagnationReport {
                kind: StagnationKind::ReadLoop,
                hint:
                    "the same resource is being re-read without progress; act on what you already read"
                        .to_string(),
            });
        }
        // "No conclusion" is only meaningful over a full window; six calls in a
        // row is a normal amount of exploration.
        if recent.len() >= WINDOW
            && progress == 0
            && failures == 0
            && repeated_reads == 0
            && recent
                .iter()
                .all(|s| matches!(s, TurnSignal::ToolCall | TurnSignal::Mutation))
        {
            return Some(StagnationReport {
                kind: StagnationKind::ReadLoop,
                hint: "tools are being called repeatedly without producing a conclusion; summarise what you learned"
                    .to_string(),
            });
        }
        None
    }
}

impl Default for StagnationDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Reduce a tool call plus its outcome to a signal.
///
/// The old classifier keyed on tool *names* that no longer exist, and lumped
/// every `files` call into `RepeatedRead`. Both halves were wrong: writes were
/// counted as reads, and nothing ever produced a failure signal, so the branch
/// that matters most — repeated failure — could not fire.
///
/// The call's arguments arrive decoded because the engine decodes them once per
/// call and hands the same object to dispatch and to the verification ledger;
/// re-decoding the wire string in each helper was four redundant parses a call.
pub fn classify_tool_call(name: &str, args: &serde_json::Value, is_error: bool) -> TurnSignal {
    if is_error {
        return TurnSignal::ToolFailure;
    }
    if is_mutating(name, args) {
        TurnSignal::Mutation
    } else {
        TurnSignal::ToolCall
    }
}

/// True when a call changes state, so it counts as progress rather than as one
/// more step in a loop.
pub fn is_mutating(name: &str, args: &serde_json::Value) -> bool {
    match name {
        // `create` and `append` were missing here, so a run that only ever
        // created files classified as a read loop — and, once plan mode exists,
        // would have been allowed to write through it.
        "files" => matches!(
            args.get("action").and_then(|v| v.as_str()).unwrap_or(""),
            "write" | "delete" | "create" | "append"
        ),
        "edit" | "terminal" | "agent" => true,
        // `todo` and `user_plan` only change per-run state: the checklist and
        // the proposal shown to the user. They write nothing to the workspace,
        // so plan mode must be able to use them — the plan prompt asks for
        // `user_plan`, and the UI renders its checklist from `todo`. Treating
        // them as mutations made plan mode refuse the two tools it exists for.
        "todo" | "user_plan" => false,
        // `git` is read-only for the inspection actions and mutating for the
        // rest, so the action decides rather than the tool name. Only the
        // actions this tool actually exposes are listed; a future write action
        // falls through to the safe default (mutating).
        "git" => !matches!(
            args.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("status"),
            "status" | "diff"
        ),
        // `memory` reads with `recall`/`search` and writes with
        // `remember`/`forget`. `knowledge` reads with `search` only.
        "memory" => !matches!(
            args.get("action").and_then(|v| v.as_str()).unwrap_or(""),
            "recall" | "search" | "search_knowledge"
        ),
        "knowledge" => !matches!(
            args.get("action").and_then(|v| v.as_str()).unwrap_or(""),
            "search"
        ),
        // A tool we have never heard of may well be mutating; assuming it is not
        // would let an unfamiliar tool's loop read as healthy work. Named tools
        // must match the registry ids: `outline` / `repo_map` were stale names
        // that no tool answers to, so a read through `symbols` fell through to
        // this default and plan mode refused it.
        "glob" | "grep" | "symbols" | "skill" | "web_fetch" | "web_fetch_pdf" | "web_search"
        | "network" | "image_vision" | "browser" | "workspace_check" => false,
        _ => true,
    }
}

/// True when a call only reads, so a repeat of it is a read-loop candidate.
pub fn is_read_only(name: &str, args: &serde_json::Value) -> bool {
    match name {
        "files" => matches!(
            args.get("action").and_then(|v| v.as_str()).unwrap_or(""),
            "read" | "list"
        ),
        "knowledge" => matches!(
            args.get("action").and_then(|v| v.as_str()).unwrap_or(""),
            "search"
        ),
        // `git status` / `git diff` inspect without changing anything. A run
        // that polls the same diff every turn is a loop worth flagging, so they
        // belong on the read-only side rather than merely the non-mutating one.
        "git" => matches!(
            args.get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("status"),
            "status" | "diff"
        ),
        "glob" | "grep" | "symbols" | "web_fetch" | "web_fetch_pdf" | "web_search"
        | "image_vision" | "network" | "workspace_check" => true,
        _ => false,
    }
}

/// A stable identity for a read, so two reads of the same file compare equal and
/// two reads of different files do not.
pub fn read_key(name: &str, args: &serde_json::Value) -> String {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    match name {
        "files" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
            format!(
                "files:{action}:{path}:{}",
                args.get("offset").unwrap_or(&serde_json::Value::Null)
            )
        }
        "glob" => format!(
            "glob:{}",
            args.get("pattern").and_then(|v| v.as_str()).unwrap_or("")
        ),
        "grep" => format!(
            "grep:{}:{}",
            args.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
            args.get("path").and_then(|v| v.as_str()).unwrap_or("")
        ),
        // A structural read is identified by the file (outline) or by the
        // symbol being looked up (references). Without this arm every outline
        // collapsed to `symbols:` and two different files looked like a repeat.
        "symbols" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("outline");
            let symbol = args
                .get("symbol")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            format!("symbols:{action}:{path}:{symbol}")
        }
        // `git status` and `git diff --staged` are different reads of the same
        // empty `path`; keying only on the path merged them.
        "git" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("status");
            let scope = args
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            format!("git:{action}:{scope}:{path}")
        }
        // The kind (`git_status` / `python_syntax` / …) is the identity of a
        // workspace check; every call has no path, so without it they all
        // compared equal.
        "workspace_check" => format!(
            "workspace_check:{}:{}",
            args.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
            path
        ),
        other => {
            // Web tools key on their URL / query; without this every search
            // collapsed to `web_search:` and the second, unrelated search was
            // flagged as a repeat of the first.
            let target = args
                .get("query")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("url").and_then(|v| v.as_str()))
                .unwrap_or(path);
            format!("{other}:{target}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers_bridge::AssistantToolCall;

    fn call(name: &str, args: serde_json::Value) -> AssistantToolCall {
        AssistantToolCall {
            id: "id".into(),
            kind: "function".into(),
            function: crate::providers_bridge::AssistantToolCallFn {
                name: name.into(),
                arguments: args.to_string(),
            },
        }
    }

    /// Mirror the engine: it decodes a call's arguments once and shares the
    /// decoded object with every reader, so the tests decode here too instead
    /// of letting each helper re-parse the wire string.
    fn decoded(call: &AssistantToolCall) -> serde_json::Value {
        serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::Value::Null)
    }

    fn classify(call: &AssistantToolCall, is_error: bool) -> TurnSignal {
        classify_tool_call(&call.function.name, &decoded(call), is_error)
    }

    fn read_only(call: &AssistantToolCall) -> bool {
        is_read_only(&call.function.name, &decoded(call))
    }

    fn mutating(call: &AssistantToolCall) -> bool {
        is_mutating(&call.function.name, &decoded(call))
    }

    fn key(call: &AssistantToolCall) -> String {
        read_key(&call.function.name, &decoded(call))
    }

    #[test]
    fn a_failed_call_is_a_failure_signal() {
        let c = call("edit", serde_json::json!({"path": "a.rs"}));
        assert_eq!(classify(&c, true), TurnSignal::ToolFailure);
    }

    #[test]
    fn a_successful_read_is_a_plain_tool_call() {
        let c = call("grep", serde_json::json!({"pattern": "fn main"}));
        assert_eq!(classify(&c, false), TurnSignal::ToolCall);
    }

    #[test]
    fn only_read_actions_count_as_read_only() {
        assert!(read_only(&call(
            "files",
            serde_json::json!({"action": "read", "path": "a.rs"})
        )));
        assert!(read_only(&call(
            "files",
            serde_json::json!({"action": "list", "path": "."})
        )));
        // This is the regression the old classifier had: a write was counted as
        // a read, so a healthy refactor tripped the read-loop branch.
        assert!(!read_only(&call(
            "files",
            serde_json::json!({"action": "write", "path": "a.rs"})
        )));
        assert!(!read_only(&call(
            "edit",
            serde_json::json!({"path": "a.rs", "old_string": "x"})
        )));
        assert!(read_only(&call(
            "glob",
            serde_json::json!({"pattern": "**/*.rs"})
        )));
    }

    #[test]
    fn read_keys_distinguish_files_and_actions() {
        let a = call(
            "files",
            serde_json::json!({"action": "read", "path": "a.rs"}),
        );
        let b = call(
            "files",
            serde_json::json!({"action": "read", "path": "b.rs"}),
        );
        let c = call(
            "files",
            serde_json::json!({"action": "read", "path": "a.rs", "offset": 1}),
        );
        assert_ne!(key(&a), key(&b));
        assert_ne!(key(&a), key(&c));
    }

    #[test]
    fn distinct_web_calls_get_distinct_keys() {
        // Every `web_search` used to collapse to `web_search:` because it has
        // no `path`, so the second, unrelated search looked like a repeat.
        let a = call("web_search", serde_json::json!({"query": "rust lifetimes"}));
        let b = call("web_search", serde_json::json!({"query": "tokio select"}));
        assert_ne!(key(&a), key(&b));
        let f1 = call("web_fetch", serde_json::json!({"url": "https://a.example"}));
        let f2 = call("web_fetch", serde_json::json!({"url": "https://b.example"}));
        assert_ne!(key(&f1), key(&f2));
    }

    #[test]
    fn planning_tools_are_allowed_in_plan_mode() {
        // Plan mode refuses `is_mutating` calls before the permission gate. The
        // plan prompt asks the model to use `user_plan`, and the UI renders the
        // checklist from `todo`, so neither may be classified as a mutation.
        for name in ["todo", "user_plan"] {
            let c = call(name, serde_json::json!({}));
            assert!(
                !mutating(&c),
                "{name} writes no workspace state and must survive plan mode"
            );
            assert!(
                !read_only(&c),
                "{name} is state, not a repeatable read of the workspace"
            );
        }
    }

    #[test]
    fn structural_reads_use_the_registry_name() {
        // The classifier used to know `outline` / `repo_map`, which are not
        // tools. `symbols` therefore fell through to "unknown means mutating"
        // and plan mode refused the one tool for inspecting code structure.
        let outline = call(
            "symbols",
            serde_json::json!({"action": "outline", "path": "a.rs"}),
        );
        assert!(!mutating(&outline));
        assert!(read_only(&outline));

        let git = call("git", serde_json::json!({"action": "status"}));
        assert!(!mutating(&git));
        assert!(read_only(&git));
        assert!(
            !mutating(&call("git", serde_json::json!({}))),
            "a missing action means `status`, matching the tool's default"
        );
        assert!(
            mutating(&call("git", serde_json::json!({"action": "commit"}))),
            "only the inspection actions of git are read-only"
        );
    }

    #[test]
    fn structural_reads_get_distinct_keys() {
        let a = call(
            "symbols",
            serde_json::json!({"action": "outline", "path": "a.rs"}),
        );
        let b = call(
            "symbols",
            serde_json::json!({"action": "outline", "path": "b.rs"}),
        );
        let c = call(
            "symbols",
            serde_json::json!({"action": "references", "symbol": "main"}),
        );
        let d = call(
            "symbols",
            serde_json::json!({"action": "references", "symbol": "run"}),
        );
        assert_ne!(key(&a), key(&b), "different files are different reads");
        assert_ne!(key(&c), key(&d), "different symbols are different reads");

        assert_ne!(
            key(&call("git", serde_json::json!({"action": "status"}))),
            key(&call(
                "git",
                serde_json::json!({"action": "diff", "scope": "staged"})
            )),
            "status and a staged diff are not the same read"
        );
        assert_ne!(
            key(&call(
                "workspace_check",
                serde_json::json!({"kind": "git_status"})
            )),
            key(&call(
                "workspace_check",
                serde_json::json!({"kind": "git_diff_check"})
            )),
        );
    }

    #[test]
    fn a_repeat_read_is_detected() {
        let mut d = StagnationDetector::new();
        assert!(!d.note_read("files:read:a.rs"));
        assert!(d.note_read("files:read:a.rs"));
    }

    #[test]
    fn repeated_failures_fire_once_half_the_window_fails() {
        let mut d = StagnationDetector::new();
        let c = call("edit", serde_json::json!({"path": "a.rs"}));
        // Nothing fires before the window is half full of failures.
        for _ in 0..(WINDOW / 2 - 1) {
            assert!(d.observe([classify(&c, true)]).is_none());
        }
        let report = d
            .observe([classify(&c, true)])
            .expect("should fire at half a window of failures");
        assert_eq!(report.kind, StagnationKind::RepeatedFailure);
    }

    #[test]
    fn a_mutating_call_is_progress_not_a_stall() {
        let edit = call("edit", serde_json::json!({"path": "a.rs"}));
        assert_eq!(
            classify(&edit, false),
            TurnSignal::Mutation,
            "an edit changes state, so it is progress"
        );
        let write = call(
            "files",
            serde_json::json!({"action": "write", "path": "a.rs"}),
        );
        assert_eq!(classify(&write, false), TurnSignal::Mutation);
        let read = call(
            "files",
            serde_json::json!({"action": "read", "path": "a.rs"}),
        );
        assert_eq!(classify(&read, false), TurnSignal::ToolCall);
    }

    #[test]
    fn an_unknown_tool_counts_as_mutating() {
        // Assuming an unfamiliar tool is read-only would let its loop read as
        // healthy work.
        let call = call("mcp__server__do_thing", serde_json::json!({}));
        assert!(mutating(&call));
    }

    #[test]
    fn provider_errors_fire() {
        let mut d = StagnationDetector::new();
        let mut fired = None;
        for _ in 0..(WINDOW + 3) {
            if let Some(report) = d.observe([TurnSignal::ProviderError, TurnSignal::Progress]) {
                fired = Some(report);
            }
        }
        assert_eq!(
            fired.expect("should fire").kind,
            StagnationKind::ProviderUnstable
        );
    }

    #[test]
    fn a_healthy_mixed_run_never_fires() {
        let mut d = StagnationDetector::new();
        let mut fired = false;
        for i in 0..(WINDOW * 6) {
            let signal = if i % 4 == 0 {
                TurnSignal::ToolCall
            } else {
                TurnSignal::Progress
            };
            if d.observe([signal]).is_some() {
                fired = true;
            }
        }
        assert!(
            !fired,
            "a run with real progress must not be flagged as stuck"
        );
    }

    #[test]
    fn a_write_heavy_run_is_not_a_read_loop() {
        let mut d = StagnationDetector::new();
        let write = call("edit", serde_json::json!({"path": "a.rs"}));
        let mut fired = false;
        for _ in 0..(WINDOW * 3) {
            if d.observe([classify(&write, false)]).is_some() {
                fired = true;
            }
        }
        assert!(!fired, "writes are progress, not a read loop");
    }

    #[test]
    fn a_short_burst_of_failures_stays_quiet() {
        let mut d = StagnationDetector::new();
        let c = call("edit", serde_json::json!({"path": "a.rs"}));
        for _ in 0..(MIN_SIGNALS - 1) {
            assert!(d.observe([classify(&c, true)]).is_none());
        }
    }

    #[test]
    fn repeated_reads_fire_on_a_half_window() {
        let mut d = StagnationDetector::new();
        let read = call(
            "files",
            serde_json::json!({"action": "read", "path": "a.rs"}),
        );
        let mut fired = None;
        for _ in 0..MIN_SIGNALS {
            if let Some(report) = d.observe([TurnSignal::RepeatedRead]) {
                fired = Some(report);
            }
        }
        assert_eq!(fired.expect("should fire").kind, StagnationKind::ReadLoop);
        let _ = read;
    }

    #[test]
    fn the_window_slides() {
        let mut d = StagnationDetector::new();
        for _ in 0..(WINDOW * 2 + 5) {
            d.observe([TurnSignal::ToolFailure]);
        }
        // Old failures must have fallen out of the window.
        assert!(d.observe([TurnSignal::Progress; WINDOW]).is_none());
    }

    #[test]
    fn an_occasional_failure_never_fires_the_detector() {
        let mut d = StagnationDetector::new();
        let c = call("edit", serde_json::json!({"path": "a.rs"}));
        let mut fired = false;
        for _ in 0..(WINDOW * 3) {
            // One failure in three is normal churn; a window that is half
            // failures is what should trigger a repair hint.
            let signals = [
                TurnSignal::Progress,
                TurnSignal::ToolCall,
                classify(&c, true),
            ];
            if d.observe(signals).is_some() {
                fired = true;
            }
        }
        assert!(!fired, "a third of calls failing is not a stall");
    }

    #[test]
    fn a_half_failing_window_does_fire() {
        let mut d = StagnationDetector::new();
        let c = call("edit", serde_json::json!({"path": "a.rs"}));
        let mut fired = false;
        for _ in 0..(WINDOW * 3) {
            if d.observe([TurnSignal::ToolCall, classify(&c, true)])
                .is_some()
            {
                fired = true;
            }
        }
        assert!(
            fired,
            "half the calls failing is a stall worth interrupting"
        );
    }
}
