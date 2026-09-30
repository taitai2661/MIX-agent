//! Permission policy engine.
//!
//! Phase 2 implements the Always Allow / Ask / Deny state machine and the
//! persistent approval records.
//!
//! Rules are evaluated with a stable precedence so the same tool can be allowed
//! globally but denied for a particular workspace, or have one pattern carve
//! out an exception over the broader rule. The order is documented next to
//! [`PermissionRule::precedence_bucket`].

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    #[default]
    Ask,
    Deny,
}

/// The scope a stored rule applies to.
///
/// `Workspace` is keyed on the run's `ToolContext::workspace_id`. `Global`
/// applies to every workspace. The legacy `Always` value is treated as an alias
/// for `Global` — see [`PermissionRule::scope`] for the normalisation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    Global,
    #[default]
    Workspace,
}

impl RuleScope {
    /// Parse the value persisted on disk, accepting legacy spellings.
    pub fn parse(value: &str) -> Self {
        match value {
            "global" | "always" => RuleScope::Global,
            _ => RuleScope::Workspace,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RuleScope::Global => "global",
            RuleScope::Workspace => "workspace",
        }
    }
}

/// A single permission rule as stored on disk.
///
/// `pattern` is a glob matched against the call's *target* (a file path for
/// `files`/`edit`/`grep`/`glob`, the executed command for `terminal`, the URL
/// for `network`/`web_fetch`). `None` matches any target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool_id: String,
    pub scope: RuleScope,
    pub decision: PermissionDecision,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl PermissionRule {
    /// Where this rule falls in the precedence ladder. Lower wins.
    ///
    /// `Workspace + pattern` is the most specific (a single workspace, a single
    /// path), and `Global + None` is the most general. Within a bucket the
    /// caller falls back to `updated_at` / id order.
    pub fn precedence_bucket(&self) -> u8 {
        match (self.scope, self.pattern.is_some()) {
            (RuleScope::Workspace, true) => 0,
            (RuleScope::Workspace, false) => 1,
            (RuleScope::Global, true) => 2,
            (RuleScope::Global, false) => 3,
        }
    }
}

