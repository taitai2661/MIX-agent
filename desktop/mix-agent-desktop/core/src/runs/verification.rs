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
}

impl VerificationLedger {
    /// Returns `true` when the call is destructive and therefore needs a
    /// confirming read before the run may conclude.
    pub fn requires_confirmation(&mut self, tool: &str, args: &serde_json::Value) -> bool {
        let Some(reason) = destructive_reason(tool, args) else {
            return false;
        };
        self.outstanding.insert(effect_key(tool, args), reason.clone());
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
            "glob" | "grep" | "web_fetch" | "web_search" | "network" | "image_vision" => {
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
    for raw in scrubbed.split(|c: char| {
        c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')' | '{' | '}')
    }) {
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

/// Tools that can serve as the confirming read.
pub fn is_read_tool(tool: &str) -> bool {
    matches!(
        tool,
        "files" | "glob" | "grep" | "image_vision" | "web_fetch" | "web_search" | "network"
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
    fn a_confirming_read_clears_only_its_own_target() {
        let mut ledger = VerificationLedger::default();
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "a.rs"})));
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "b.rs"})));
        assert_eq!(ledger.outstanding().len(), 2);

        ledger.confirm("files", &args(serde_json::json!({"action": "read", "path": "a.rs"})));
        assert_eq!(
            ledger.outstanding(),
            vec!["path:b.rs".to_string()],
            "reading a.rs must not clear the gate for b.rs"
        );

        ledger.confirm("files", &args(serde_json::json!({"action": "read", "path": "b.rs"})));
        assert!(!ledger.is_blocked());
    }

    #[test]
    fn a_confirming_read_clears_the_gate() {
        let mut ledger = VerificationLedger::default();
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "a.rs"})));
        assert!(ledger.is_blocked());
        ledger.confirm("files", &args(serde_json::json!({"action": "read", "path": "a.rs"})));
        assert!(!ledger.is_blocked(), "the read must actually clear the gate");
    }

    #[test]
    fn a_write_does_not_clear_the_gate() {
        let mut ledger = VerificationLedger::default();
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "a.rs"})));
        ledger.confirm("edit", &args(serde_json::json!({"path": "a.rs"})));
        assert!(ledger.is_blocked(), "another write is not a confirming read");
    }

    #[test]
    fn unknown_tools_are_never_destructive() {
        let mut ledger = VerificationLedger::default();
        assert!(!ledger.requires_confirmation("browser", &args(serde_json::json!({}))));
        assert!(!ledger.is_blocked());
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
