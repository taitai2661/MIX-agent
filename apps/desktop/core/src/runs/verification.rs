//! Strict verification gating for destructive tool calls.
//!
//! After a write or a delete, the agent should read the result back before it
//! concludes anything. This module records which operations left an unconfirmed
//! effect and decides whether a confirming read is still outstanding.
//!
//! The ledger is keyed by the *target* of the effect (a path, or a shell
//! command), not by the tool name. Keying on the tool meant editing `a.rs` and
//! `b.rs` left a single `edit` entry, so reading `a.rs` back cleared the gate
//! for `b.rs` too. A confirming read now clears only the targets it actually
//! covers.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationGate {
    pub phase: String,
    pub required_tools: Vec<String>,
}

/// Commands whose bare name makes a `terminal` call destructive.
const DESTRUCTIVE_COMMANDS: &[&str] = &[
    "rm", "rmdir", "mv", "dd", "mkfs", "chmod", "chown", "chgrp", "kill", "pkill", "killall",
    "truncate", "shred", "tee", "unlink",
];

/// Multi-word fixtures that a token walk would miss.
const DESTRUCTIVE_GIT: &[&str] = &[
    "git reset",
    "git clean",
    "git checkout",
    "git restore",
    "git branch -d",
    "git branch -D",
    "git push --force",
    "git push -f",
];

#[derive(Default)]
pub struct VerificationLedger {
    /// Outstanding effects, each mapped to the human-readable reason the effect
    /// is destructive. Keyed by target so unrelated paths stay independent.
    outstanding: HashMap<String, String>,
    last_reason: Option<String>,
    /// The project's own check (`cargo test`, `pnpm test`, …). `None` disables
    /// the second gate entirely: with nothing configured there is nothing to
    /// run, and demanding a check that does not exist would stall every run.
    verify_command: Option<String>,
    /// Paths changed since the last successful `verify_command`. This is the
    /// second, independent question from `outstanding`: that one asks "did you
    /// read the effect back?", this one asks "did you run the project's own
    /// check?". A file can be read back correctly and still be broken.
    unverified: std::collections::BTreeSet<String>,
}

impl VerificationLedger {
    /// A ledger that also gates on the project's verification command.
    pub fn new(verify_command: Option<String>) -> Self {
        Self {
            verify_command: verify_command.filter(|cmd| !cmd.trim().is_empty()),
            ..Default::default()
        }
    }

    /// The configured check, for the repair message and the event payload.
    pub fn verify_command(&self) -> Option<&str> {
        self.verify_command.as_deref()
    }

    /// Record that a file was changed, which re-arms the check gate.
    ///
    /// Called only for changes the agent made deliberately through `edit` or
    /// `files.write` — not for whatever a shell command happened to touch. A
    /// command that rewrites files (`cargo fmt`, `npm install`) would otherwise
    /// re-arm the gate that running that very command is supposed to clear, and
    /// the run could never finish.
    pub fn note_mutation(&mut self, path: &str) {
        if self.verify_command.is_none() || path.is_empty() {
            return;
        }
        self.unverified.insert(path.to_string());
    }

    /// Record a shell command the agent ran.
    ///
    /// A successful run clears the gate only when it *is* the configured check —
    /// otherwise any successful command (`ls`, `echo`) would count as
    /// verification, which is how a gate like this usually fails open in
    /// practice.
    pub fn note_command(&mut self, command: &str, exit_ok: bool) {
        if !exit_ok {
            return;
        }
        let Some(configured) = self.verify_command.as_deref() else {
            return;
        };
        if command_matches_variant(command, configured) {
            self.unverified.clear();
        }
    }

    /// `true` when a change is still waiting for the project's check.
    pub fn needs_verification(&self) -> bool {
        self.verify_command.is_some() && !self.unverified.is_empty()
    }

    /// The changed paths still waiting for a check, sorted.
    pub fn unverified(&self) -> Vec<String> {
        self.unverified.iter().cloned().collect()
    }

    /// Returns `true` when the call is destructive and therefore needs a
    /// confirming read before the run may conclude.
    pub fn requires_confirmation(&mut self, tool: &str, args: &serde_json::Value) -> bool {
        let Some(reason) = destructive_reason(tool, args) else {
            return false;
        };
        // A batch `edit` carries its targets in `edits`, not in `path`. Keying
        // the whole call on `path` produced one `path:` (empty) entry, which
        // reading either file could never clear — the run then failed
        // verification no matter how correct the edit was. Arm every target so
        // a confirming read clears exactly what it covers.
        let targets = mutated_paths(tool, args);
        if targets.is_empty() {
            self.outstanding
                .insert(effect_key(tool, args), reason.clone());
        } else {
            for path in targets {
                self.outstanding
                    .insert(format!("path:{path}"), reason.clone());
            }
        }
        self.last_reason = Some(reason);
        true
    }

    /// A confirming read clears the target(s) it actually covers.
    pub fn confirm(&mut self, tool: &str, args: &serde_json::Value) {
        match tool {
            "files" => {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
                if action == "read" || action == "list" {
                    match path_arg(args) {
                        Some(path) => {
                            self.outstanding.remove(&format!("path:{path}"));
                        }
                        // A bare listing confirms the whole tree.
                        None => self.outstanding.clear(),
                    }
                }
            }
            // Broad searches and fetches are not path-specific; treat them as
            // confirming the outstanding set rather than pinning the agent.
            "glob" | "grep" | "web_fetch" | "web_fetch_pdf" | "web_search" | "network"
            | "image_vision" | "workspace_check" => {
                self.outstanding.clear();
            }
            // A search over knowledge confirms the outstanding set; any other
            // knowledge action (ingest / delete) is path-specific and must not.
            "knowledge" if args.get("action").and_then(|v| v.as_str()) == Some("search") => {
                self.outstanding.clear();
            }
            _ => {}
        }
    }

    pub fn is_blocked(&self) -> bool {
        !self.outstanding.is_empty()
    }

    pub fn outstanding(&self) -> Vec<String> {
        let mut out: Vec<String> = self.outstanding.keys().cloned().collect();
        out.sort();
        out
    }

    pub fn last_reason(&self) -> Option<String> {
        self.last_reason.clone()
    }
}

fn path_arg(args: &serde_json::Value) -> Option<&str> {
    args.get("path").and_then(|v| v.as_str())
}

fn command_arg(args: &serde_json::Value) -> Option<&str> {
    args.get("command").and_then(|v| v.as_str())
}

/// A stable key for the effect a destructive call leaves behind.
fn effect_key(tool: &str, args: &serde_json::Value) -> String {
    match tool {
        "terminal" => format!("cmd:{}", command_arg(args).unwrap_or_default()),
        _ => format!("path:{}", path_arg(args).unwrap_or_default()),
    }
}

/// `true` when `command` runs `configured`, with or without extra arguments.
///
/// Substring matching alone would let `pnpm test:unit` satisfy a configured
/// `pnpm test`, so the match has to end at a command boundary — whitespace, a
/// shell separator, or the end of the string. A prefix like `cd x && ` and a
/// suffix like ` -- --nocapture` both still count, which is what makes this
/// usable at all: the model rarely retypes the command character for character.
fn command_matches_variant(command: &str, configured: &str) -> bool {
    let configured = configured.trim();
    if configured.is_empty() {
        return false;
    }
    let command = command.trim();
    let mut from = 0usize;
    while let Some(offset) = command[from..].find(configured) {
        let start = from + offset;
        let end = start + configured.len();
        let boundary = command[end..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || matches!(c, ';' | '|' | '&' | ')' | '"' | '\''));
        if boundary {
            return true;
        }
        from = end;
    }
    false
}

/// Files a Tool call changed, so the engine can arm the check gate.
///
/// `edit` may carry a batch of `edits` entries, so this returns a list rather
/// than one path; `files` only counts its mutating actions, because a `read`
/// that armed the gate would leave the run permanently unable to finish.
pub fn mutated_paths(tool: &str, args: &serde_json::Value) -> Vec<String> {
    match tool {
        "edit" => {
            if let Some(batch) = args.get("edits").and_then(|v| v.as_array()) {
                return batch
                    .iter()
                    .filter_map(|entry| entry.get("path").and_then(|v| v.as_str()))
                    .map(str::to_string)
                    .collect();
            }
            path_arg(args).map(str::to_string).into_iter().collect()
        }
        "files" => match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "write" | "create" | "delete" | "append" => {
                path_arg(args).map(str::to_string).into_iter().collect()
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The exit status a `terminal` Tool result reported, when it reported one.
pub fn exit_code_of(result: &serde_json::Value) -> Option<i64> {
    result.get("exit_code").and_then(serde_json::Value::as_i64)
}

/// Human-readable reason a call is destructive, or `None` when it is not.
pub fn destructive_reason(tool: &str, args: &serde_json::Value) -> Option<String> {
    match tool {
        "files" => match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "delete" => Some("files.delete removed content".to_string()),
            "write" => Some("files.write replaced a whole file".to_string()),
            _ => None,
        },
        "edit" => Some("edit changed a file on disk".to_string()),
        "terminal" => shell_destructive_reason(command_arg(args).unwrap_or("")),
        _ => None,
    }
}

/// Detect a destructive shell command without the substring false positives of
/// the previous fragment list (where `"rm "` matched `"confirm "`).
pub fn shell_destructive_reason(command: &str) -> Option<String> {
    // File-descriptor duplication (`2>&1`, `&>`) is the common redirection that
    // is *not* a write to a file, so scrub it before looking for `>`.
    let scrubbed = command.replace("2>&1", " ").replace("&>", " ");
    if scrubbed.contains(">>") || scrubbed.contains('>') {
        return Some("shell output redirection".to_string());
    }
    for pattern in DESTRUCTIVE_GIT {
        if command.contains(pattern) {
            return Some(format!("destructive git command ({pattern})"));
        }
    }
    for raw in scrubbed
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')' | '{' | '}'))
    {
        let token = raw.trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | '$' | '(' | ')'));
        if token.is_empty() {
            continue;
        }
        if DESTRUCTIVE_COMMANDS.contains(&token) {
            return Some(format!("destructive command ({token})"));
        }
        if token == "-delete" || token == "--delete" {
            return Some("find -delete".to_string());
        }
    }
    None
}

/// Privilege-escalation prefixes, which the app never runs.
///
/// `files`, `edit`, `glob` and `grep` are confined to the workspace, but a
/// shell command is not: it can `cd /` and write anywhere the user can. This
/// cannot be fixed by confinement without a sandbox, so the next best thing is
/// to notice the two shapes that actually matter — a write outside the
/// workspace, and an escalation wrapper — and surface them for approval.
const ESCALATION_COMMANDS: &[&str] = &["sudo", "doas", "pkexec", "su"];

/// A privilege-escalation command inside `command`, if any.
pub fn shell_escalation(command: &str) -> Option<String> {
    for raw in command
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')' | '{' | '}'))
    {
        let token = raw.trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | '$'));
        if ESCALATION_COMMANDS.contains(&token) {
            return Some(token.to_string());
        }
    }
    None
}

/// The path a write target points at, resolved against `root`.
///
/// Returns `None` for a target that cannot be judged — `~` expansion, a shell
/// variable, a bare `&1`-style descriptor — because guessing there produces
/// more false alarms than findings.
fn resolve_shell_path(target: &str, root: &std::path::Path) -> Option<std::path::PathBuf> {
    let target = target
        .trim()
        .trim_matches(|c: char| matches!(c, '\'' | '"'));
    if target.is_empty() || target.starts_with('~') || target.starts_with('$') {
        return None;
    }
    if matches!(
        target,
        "&1" | "&2" | "-" | "/dev/null" | "/dev/stdout" | "/dev/stderr"
    ) {
        return None;
    }
    let path = std::path::Path::new(target);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    Some(lexical_clean(&absolute))
}

/// Drop `.` and resolve `..` without touching the filesystem.
///
/// `canonicalize` cannot be used on a target that does not exist yet, and a
/// write target usually does not.
fn lexical_clean(path: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// File targets a shell command writes to: redirections and `tee` arguments.
fn write_targets(command: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut chars = command.chars().peekable();
    let mut token = String::new();
    let mut after_tee = false;
    while let Some(c) = chars.next() {
        if c == '>' {
            // `>>` is the same target as `>`.
            if chars.peek() == Some(&'>') {
                chars.next();
            }
            let mut target = String::new();
            for next in chars.by_ref() {
                if next.is_whitespace() {
                    if !target.is_empty() {
                        break;
                    }
                    continue;
                }
                if matches!(next, ';' | '|' | '&') && !target.is_empty() {
                    break;
                }
                target.push(next);
            }
            if !target.is_empty() {
                targets.push(target);
            }
            token.clear();
            continue;
        }
        if c.is_whitespace() || matches!(c, ';' | '|' | '&') {
            if after_tee && !token.is_empty() && !token.starts_with('-') {
                targets.push(token.clone());
            }
            after_tee = token == "tee" || token.ends_with("/tee");
            token.clear();
            continue;
        }
        token.push(c);
    }
    if after_tee && !token.is_empty() && !token.starts_with('-') {
        targets.push(token);
    }
    targets
}

/// A write outside the workspace root, reported as the offending target.
///
/// Relative targets are resolved against the workspace root, which is the
/// shell's starting directory. A command that `cd`s elsewhere first can move
/// its writes out of view; that case is not detectable without executing the
/// command, so this deliberately errs towards under-reporting.
pub fn shell_writes_outside(command: &str, root: &std::path::Path) -> Option<String> {
    let root = root.canonicalize().unwrap_or_else(|_| lexical_clean(root));
    let scrubbed = command.replace("2>&1", " ").replace("2>&", " ");
    for target in write_targets(&scrubbed) {
        match resolve_shell_path(&target, &root) {
            Some(path) if !path.starts_with(&root) => return Some(target),
            _ => {}
        }
    }
    None
}

/// Tools that can serve as the confirming read.
pub fn is_read_tool(tool: &str) -> bool {
    matches!(
        tool,
        "files"
            | "glob"
            | "grep"
            | "image_vision"
            | "web_fetch"
            | "web_fetch_pdf"
            | "web_search"
            | "network"
            | "workspace_check"
            | "knowledge"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: serde_json::Value) -> serde_json::Value {
        value
    }

    #[test]
    fn a_write_leaves_an_outstanding_gate() {
        let mut ledger = VerificationLedger::default();
        assert!(ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "a.rs"}))));
        assert!(ledger.is_blocked());
        assert_eq!(ledger.outstanding(), vec!["path:a.rs".to_string()]);
    }

    #[test]
    fn a_read_leaves_no_gate() {
        let mut ledger = VerificationLedger::default();
        assert!(!ledger.requires_confirmation("grep", &args(serde_json::json!({"pattern": "x"}))));
        assert!(!ledger.is_blocked());
    }

    #[test]
    fn a_files_read_leaves_no_gate_but_a_write_does() {
        let mut ledger = VerificationLedger::default();
        assert!(!ledger.requires_confirmation(
            "files",
            &args(serde_json::json!({"action": "read", "path": "a.rs"}))
        ));
        assert!(ledger.requires_confirmation(
            "files",
            &args(serde_json::json!({"action": "write", "path": "a.rs"}))
        ));
        assert!(ledger.is_blocked());
    }

    #[test]
    fn a_destructive_shell_command_is_flagged() {
        assert!(shell_destructive_reason("rm -rf build").is_some());
        assert!(shell_destructive_reason("git reset --hard HEAD~5").is_some());
        assert!(shell_destructive_reason("cargo test --workspace > out.log").is_some());
        assert!(shell_destructive_reason("find . -name '*.tmp' -delete").is_some());
    }

    #[test]
    fn a_benign_shell_command_is_not_flagged() {
        assert!(shell_destructive_reason("cargo test --workspace").is_none());
        assert!(shell_destructive_reason("git status").is_none());
        // The old substring list flagged `confirm`, `form`, `perform` as `rm`.
        assert!(shell_destructive_reason("confirm the deployment").is_none());
        assert!(shell_destructive_reason("./perform_checks.sh").is_none());
        // fd duplication is not a file write.
        assert!(shell_destructive_reason("make 2>&1 | tail -n 20").is_none());
    }

    #[test]
    fn a_batch_edit_arms_every_target_not_one_empty_path() {
        let mut ledger = VerificationLedger::default();
        assert!(ledger.requires_confirmation(
            "edit",
            &args(serde_json::json!({
                "edits": [
                    {"path": "src/a.rs", "old_string": "a", "new_string": "b"},
                    {"path": "src/b.rs", "old_string": "c", "new_string": "d"}
                ]
            }))
        ));
        assert_eq!(
            ledger.outstanding(),
            vec!["path:src/a.rs".to_string(), "path:src/b.rs".to_string()]
        );

        // Reading one target leaves the other outstanding; the old keying
        // stored `path:` and neither read could ever clear it.
        ledger.confirm(
            "files",
            &args(serde_json::json!({"action": "read", "path": "src/a.rs"})),
        );
        assert_eq!(ledger.outstanding(), vec!["path:src/b.rs".to_string()]);
        ledger.confirm(
            "files",
            &args(serde_json::json!({"action": "read", "path": "src/b.rs"})),
        );
        assert!(!ledger.is_blocked());
    }

    #[test]
    fn a_confirming_read_clears_only_its_own_target() {
        let mut ledger = VerificationLedger::default();
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "a.rs"})));
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "b.rs"})));
        assert_eq!(ledger.outstanding().len(), 2);

        ledger.confirm(
            "files",
            &args(serde_json::json!({"action": "read", "path": "a.rs"})),
        );
        assert_eq!(
            ledger.outstanding(),
            vec!["path:b.rs".to_string()],
            "reading a.rs must not clear the gate for b.rs"
        );

        ledger.confirm(
            "files",
            &args(serde_json::json!({"action": "read", "path": "b.rs"})),
        );
        assert!(!ledger.is_blocked());
    }

    #[test]
    fn a_confirming_read_clears_the_gate() {
        let mut ledger = VerificationLedger::default();
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "a.rs"})));
        assert!(ledger.is_blocked());
        ledger.confirm(
            "files",
            &args(serde_json::json!({"action": "read", "path": "a.rs"})),
        );
        assert!(
            !ledger.is_blocked(),
            "the read must actually clear the gate"
        );
    }

    #[test]
    fn a_write_does_not_clear_the_gate() {
        let mut ledger = VerificationLedger::default();
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "a.rs"})));
        ledger.confirm("edit", &args(serde_json::json!({"path": "a.rs"})));
        assert!(
            ledger.is_blocked(),
            "another write is not a confirming read"
        );
    }

    #[test]
    fn unknown_tools_are_never_destructive() {
        let mut ledger = VerificationLedger::default();
        assert!(!ledger.requires_confirmation("browser", &args(serde_json::json!({}))));
        assert!(!ledger.is_blocked());
    }

    #[test]
    fn a_mutation_arms_the_check_gate_and_the_check_clears_it() {
        let mut ledger = VerificationLedger::new(Some("pnpm rust:test".into()));
        ledger.note_mutation("src/lib.rs");
        assert!(ledger.needs_verification());
        assert_eq!(ledger.unverified(), vec!["src/lib.rs".to_string()]);

        ledger.note_command("pnpm rust:test", true);
        assert!(!ledger.needs_verification());
    }

    #[test]
    fn an_unrelated_or_failing_command_does_not_clear_the_gate() {
        let mut ledger = VerificationLedger::new(Some("pnpm rust:test".into()));
        ledger.note_mutation("src/lib.rs");

        ledger.note_command("ls -la", true);
        assert!(ledger.needs_verification(), "`ls` is not a check");

        ledger.note_command("pnpm rust:test", false);
        assert!(
            ledger.needs_verification(),
            "a failing check must not clear the gate"
        );
    }

    #[test]
    fn the_gate_is_inert_without_a_configured_command() {
        let mut ledger = VerificationLedger::new(None);
        ledger.note_mutation("src/lib.rs");
        assert!(!ledger.needs_verification());
        assert!(ledger.unverified().is_empty());

        // A blank command is the same as none: there is nothing to run.
        let mut blank = VerificationLedger::new(Some("   ".into()));
        blank.note_mutation("src/lib.rs");
        assert!(!blank.needs_verification());
    }

    #[test]
    fn extra_arguments_still_satisfy_the_configured_command() {
        let mut ledger = VerificationLedger::new(Some("pnpm rust:test".into()));
        ledger.note_mutation("src/lib.rs");
        ledger.note_command(
            "cd mix-agent-desktop && pnpm rust:test -- --nocapture",
            true,
        );
        assert!(!ledger.needs_verification());
    }

    #[test]
    fn a_different_script_with_the_configured_prefix_does_not_satisfy_it() {
        let mut ledger = VerificationLedger::new(Some("pnpm test".into()));
        ledger.note_mutation("src/lib.rs");
        ledger.note_command("pnpm test:unit", true);
        assert!(
            ledger.needs_verification(),
            "`pnpm test:unit` is not the configured `pnpm test`"
        );
    }

    #[test]
    fn an_edit_batch_arms_every_path_it_touches() {
        let paths = mutated_paths(
            "edit",
            &serde_json::json!({
                "edits": [
                    {"path": "a.rs", "old_string": "x", "new_string": "y"},
                    {"path": "b.rs", "old_string": "x", "new_string": "y"}
                ]
            }),
        );
        assert_eq!(paths, vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn a_file_read_does_not_arm_the_gate_but_a_write_does() {
        assert!(mutated_paths(
            "files",
            &serde_json::json!({"action": "read", "path": "a.rs"})
        )
        .is_empty());
        assert_eq!(
            mutated_paths(
                "files",
                &serde_json::json!({"action": "write", "path": "a.rs"})
            ),
            vec!["a.rs"]
        );
    }

    #[test]
    fn a_write_inside_the_workspace_is_allowed() {
        let root = std::path::Path::new("/tmp/mixd-escape-in");
        assert!(shell_writes_outside("cargo test > out.log", root).is_none());
        assert!(shell_writes_outside("printf x > src/generated.rs", root).is_none());
    }

    #[test]
    fn a_write_outside_the_workspace_is_reported() {
        let root = std::path::Path::new("/tmp/mixd-escape-out");
        assert_eq!(
            shell_writes_outside("echo pwned > /etc/hosts", root),
            Some("/etc/hosts".to_string())
        );
        assert_eq!(
            shell_writes_outside("echo pwned > ../../outside.txt", root),
            Some("../../outside.txt".to_string())
        );
        assert_eq!(
            shell_writes_outside("echo x | tee /tmp/elsewhere.log", root),
            Some("/tmp/elsewhere.log".to_string())
        );
    }

    #[test]
    fn descriptor_redirection_is_not_a_file_write() {
        let root = std::path::Path::new("/tmp/mixd-escape-fd");
        assert!(shell_writes_outside("make 2>&1 | tail -n 20", root).is_none());
        assert!(shell_writes_outside("cargo test > /dev/null", root).is_none());
    }

    #[test]
    fn an_escalation_wrapper_is_reported() {
        assert_eq!(
            shell_escalation("sudo rm -rf build"),
            Some("sudo".to_string())
        );
        assert!(shell_escalation("cargo build").is_none());
    }

    #[test]
    fn git_wipe_commands_are_flagged() {
        assert!(destructive_reason(
            "terminal",
            &serde_json::json!({"command": "git reset --hard HEAD~5"})
        )
        .is_some());
        assert!(
            destructive_reason("terminal", &serde_json::json!({"command": "git status"})).is_none()
        );
    }
}
