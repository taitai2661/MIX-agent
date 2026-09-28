//! Strict verification gating for destructive tool calls.
//!
//! After a write or a delete, the agent should read the result back before it
//! concludes anything. This module records which operations left an unconfirmed
//! effect and decides whether a confirming read is still outstanding.
//!
//! Earlier this ledger was write-only: `requires_confirmation` inserted into a
//! set that `is_blocked` and `confirm` never read from, so the `verification_required`
//! event fired and nothing ever acted on it. The tool list also still referenced
//! `delete_file` / `write_file`, which no longer exist, so only two of the
//! destructive paths were even recognised.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationGate {
    pub phase: String,
    pub required_tools: Vec<String>,
}

/// Tools whose effects must be confirmed by a follow-up read.
const DESTRUCTIVE_TOOLS: &[&str] = &["files", "edit", "terminal"];

/// Shell fragments that make a `terminal` call destructive. Kept as substrings
/// because a full parse would need to model quoting, chaining and substitution,
/// and the failure mode of a miss is a missed warning, not a wrong action.
const DESTRUCTIVE_SHELL_FRAGMENTS: &[&str] = &[
    "rm ",
    "rm\t",
    "rm -",
    "mv ",
    "dd ",
    "> ",
    ">>",
    "mkfs",
    "chmod",
    "chown",
    "kill",
    "truncate",
    "git reset",
    "git clean",
    "git checkout",
];

#[derive(Default)]
pub struct VerificationLedger {
    outstanding: HashSet<String>,
    last_reason: Option<String>,
}

impl VerificationLedger {
    /// Returns `true` when the call is destructive and therefore needs a
    /// confirming read before the run may conclude.
    pub fn requires_confirmation(&mut self, tool: &str, args: &serde_json::Value) -> bool {
        let Some(reason) = destructive_reason(tool, args) else {
            return false;
        };
        self.outstanding.insert(tool.to_string());
        self.last_reason = Some(reason);
        true
    }

    /// A confirming read clears the outstanding gate.
    pub fn confirm(&mut self, tool: &str) {
        if is_read_tool(tool) {
            self.outstanding.clear();
        }
    }

    pub fn is_blocked(&self) -> bool {
        !self.outstanding.is_empty()
    }

    pub fn outstanding(&self) -> Vec<String> {
        let mut out: Vec<String> = self.outstanding.iter().cloned().collect();
        out.sort();
        out
    }

    pub fn last_reason(&self) -> Option<String> {
        self.last_reason.clone()
    }
}

/// Human-readable reason a call is destructive, or `None` when it is not.
pub fn destructive_reason(tool: &str, args: &serde_json::Value) -> Option<String> {
    if !DESTRUCTIVE_TOOLS.contains(&tool) {
        return None;
    }
    match tool {
        "files" => match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "delete" => Some("files.delete removed content".to_string()),
            "write" => Some("files.write replaced a whole file".to_string()),
            _ => None,
        },
        "edit" => Some("edit changed a file on disk".to_string()),
        "terminal" => {
            let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            DESTRUCTIVE_SHELL_FRAGMENTS
                .iter()
                .find(|fragment| command.contains(**fragment))
                .map(|fragment| format!("terminal ran a destructive command ({fragment:?})"))
        }
        _ => None,
    }
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
        assert_eq!(ledger.outstanding(), vec!["edit".to_string()]);
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
        let mut ledger = VerificationLedger::default();
        assert!(ledger.requires_confirmation(
            "terminal",
            &args(serde_json::json!({"command": "rm -rf build"}))
        ));
        assert!(ledger
            .last_reason()
            .expect("reason")
            .contains("destructive command"));
    }

    #[test]
    fn a_benign_shell_command_is_not_flagged() {
        let mut ledger = VerificationLedger::default();
        assert!(!ledger.requires_confirmation(
            "terminal",
            &args(serde_json::json!({"command": "cargo test --workspace"}))
        ));
        assert!(!ledger.is_blocked());
    }

    #[test]
    fn a_confirming_read_clears_the_gate() {
        let mut ledger = VerificationLedger::default();
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "a.rs"})));
        assert!(ledger.is_blocked());
        ledger.confirm("files");
        assert!(
            !ledger.is_blocked(),
            "the read must actually clear the gate"
        );
    }

    #[test]
    fn a_write_does_not_clear_the_gate() {
        let mut ledger = VerificationLedger::default();
        ledger.requires_confirmation("edit", &args(serde_json::json!({"path": "a.rs"})));
        ledger.confirm("edit");
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
