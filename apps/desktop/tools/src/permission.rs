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

/// Validate a rule pattern with the same parser the executor uses for matching.
///
/// Keeping this next to the matcher prevents the settings UI and runtime from
/// disagreeing about which patterns are usable.
pub fn is_valid_pattern(pattern: &str) -> bool {
    glob::Pattern::new(pattern).is_ok()
}

/// Stable digest for a *remembered* call target.
///
/// "Always Allow" used to persist the raw target — for `terminal` that is the
/// whole shell command, including any credential typed into it. A rule only
/// needs to know whether a later call has the same target, so we store this
/// non-reversible digest instead and keep a redacted copy for display. Two
/// independent FNV-1a passes give 128 bits without pulling in a hash crate;
/// this is a matching key, not a cryptographic secret store.
pub fn pattern_digest(value: &str) -> String {
    fn fnv(bytes: &[u8], seed: u64) -> u64 {
        let mut hash = seed;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash
    }
    let bytes = value.as_bytes();
    format!(
        "{:016x}{:016x}",
        fnv(bytes, 0xcbf2_9ce4_8422_2325),
        fnv(bytes, 0x8422_2325_cbf2_9ce4)
    )
}

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
    /// Lives in memory until the app exits. Never written to the database.
    Session,
}

impl RuleScope {
    /// Parse the value persisted on disk, accepting legacy spellings.
    pub fn parse(value: &str) -> Self {
        match value {
            "global" | "always" => RuleScope::Global,
            "session" => RuleScope::Session,
            _ => RuleScope::Workspace,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RuleScope::Global => "global",
            RuleScope::Workspace => "workspace",
            RuleScope::Session => "session",
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
    /// When present, matching uses this digest of the raw target rather than
    /// the (redacted, display-only) `pattern` glob.
    #[serde(default)]
    pub pattern_digest: Option<String>,
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
        // Approval-driven rules keep a digest of the raw target and only a
        // redacted (possibly empty) glob for display. Such a rule is still
        // target-scoped, so it must not fall into the generic bucket where a
        // broader rule could override it.
        let targeted = self.pattern.is_some() || self.pattern_digest.is_some();
        match (self.scope, targeted) {
            (RuleScope::Session, true) => 0,
            (RuleScope::Session, false) => 1,
            (RuleScope::Workspace, true) => 2,
            (RuleScope::Workspace, false) => 3,
            (RuleScope::Global, true) => 4,
            (RuleScope::Global, false) => 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(scope: RuleScope, pattern: Option<&str>, digest: Option<&str>) -> PermissionRule {
        PermissionRule {
            tool_id: "terminal".into(),
            scope,
            decision: PermissionDecision::Deny,
            pattern: pattern.map(str::to_string),
            pattern_digest: digest.map(str::to_string),
            workspace_id: None,
            updated_at: None,
        }
    }

    #[test]
    fn digest_without_display_pattern_keeps_its_specificity() {
        // A redacted command can leave no display glob at all. The rule still
        // targets one remembered call, so it must outrank a generic rule.
        let specific = rule(RuleScope::Workspace, None, Some("digest"));
        let generic = rule(RuleScope::Workspace, None, None);
        assert!(specific.precedence_bucket() < generic.precedence_bucket());
        assert_eq!(specific.precedence_bucket(), 2);
    }
}
