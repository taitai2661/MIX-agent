//! Serialised executor for side-effecting Tool calls: the approval queue, the
//! permission-rule resolver, and the persistent approval storage.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::approval::{ApprovalOutcome, PendingApproval};
use crate::permission::{PermissionDecision, PermissionRule, RuleScope};
use crate::ToolResult;
use chrono::Utc;
use mixd_storage::policy::{PolicyDecision as FileDecision, PolicyFile};
use mixd_storage::{SecretStore, Storage};
use serde_json::json;
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

/// How long an approval prompt stays answerable before the Run resumes and the
/// Tool is treated as denied.
///
/// Desktop runs are often left in the background while the user is in another
/// app, so the old fifteen-minute deadline turned "I stepped away" into a
/// silently denied Tool. The default is now a day and the value is configurable
/// from Settings; the prompt still auto-denies, so an abandoned Run cannot hang
/// forever.
pub const DEFAULT_APPROVAL_TTL_SECS: i64 = 24 * 60 * 60;
/// Never allow a zero/negative TTL: the engine would deny the prompt instantly.
pub const MIN_APPROVAL_TTL_SECS: i64 = 60;
/// A week is the practical ceiling; beyond that an abandoned prompt is
/// indistinguishable from a leaked one.
pub const MAX_APPROVAL_TTL_SECS: i64 = 7 * 24 * 60 * 60;
/// Bounds for one "extend" click. A single click adds at most a day, while the
/// cumulative deadline is still capped by [`MAX_APPROVAL_TTL_SECS`] (a week),
/// so a prompt can never be extended past the point where an abandoned Run is
/// indistinguishable from a leaked one.
pub const MIN_APPROVAL_EXTENSION_SECS: i64 = 60;
pub const MAX_APPROVAL_EXTENSION_SECS: i64 = 24 * 60 * 60;

/// A paused Run's wake-up channel, kept together with the prompt's deadline.
///
/// The engine stops waiting when the TTL elapses but never calls
/// [`ToolExecutor::resolve_approval`] (its `select!` timeout branch only emits
/// the denial), so without a recorded expiry neither this channel nor the
/// `pending` row in `approvals` would ever be reclaimed.
struct Waiter {
    tx: oneshot::Sender<ApprovalOutcome>,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct ToolExecutor {
    pub storage: Arc<Storage>,
    pub secrets: Arc<SecretStore>,
    pub pending: Arc<Mutex<Vec<PendingApproval>>>,
    /// Wakes the paused Run when the user answers a prompt.
    waiters: Arc<Mutex<HashMap<String, Waiter>>>,
    /// "Allow for this session" rules. Deliberately in memory: a session grant
    /// must not survive a restart or leak into another workspace.
    session_rules: Arc<Mutex<Vec<PermissionRule>>>,
    /// How long a new approval prompt stays answerable. Shared so a Settings
    /// change applies to prompts created afterwards without rebuilding the
    /// engine or the registry.
    approval_ttl_secs: Arc<AtomicI64>,
    /// Non-interactive policy for the headless CLI.
    ///
    /// `None` keeps the normal prompt. `Some(true)` approves every call and
    /// `Some(false)` denies every call immediately, so a CLI run cannot park
    /// for 24 hours on a prompt no one can answer. The GUI never sets this.
    headless_approval: Arc<std::sync::Mutex<Option<bool>>>,
    /// Workspace `.mix/policy.toml`, when one has been deployed. Loaded at
    /// startup (see [`Self::new`]) and replaceable after a Settings edit.
    /// `None` keeps the legacy tool-default fallback; a present file always
    /// evaluates fail-closed through `evaluate_tool_policy`.
    policy_file: Arc<std::sync::RwLock<Option<Arc<PolicyFile>>>>,
    /// Serialises approval resolution so "still pending?" checks and the
    /// state/rule/audit mutations that follow cannot interleave with a second
    /// decision (or the expiry reaper) for the same prompt.
    approval_resolve_lock: Arc<Mutex<()>>,
}

/// The information [`ToolExecutor::resolve_permission`] needs to evaluate a
/// stored rule against the call that's about to be made.
#[derive(Debug, Clone, Default)]
pub struct PermissionContext<'a> {
    /// Workspace the run is bound to; `None` means "no workspace" (e.g. an
    /// ad-hoc chat) and matches only `Global` rules.
    pub workspace_id: Option<Uuid>,
    /// The string the tool wants the rule's `pattern` compared against. Most
    /// tools expose a path or URL; tools that have no useful target should
    /// leave this `None`.
    pub pattern_target: Option<&'a str>,
}

fn parse_rule(data: &serde_json::Value) -> Option<PermissionRule> {
    let tool_id = data.get("toolId").and_then(|v| v.as_str())?.to_string();
    let decision = match data.get("decision").and_then(|v| v.as_str()) {
        Some("allow") => PermissionDecision::Allow,
        Some("deny") => PermissionDecision::Deny,
        Some("ask") => PermissionDecision::Ask,
        _ => return None,
    };
    let scope = match data.get("scope").and_then(|v| v.as_str()) {
        Some(s) => RuleScope::parse(s),
        None => RuleScope::default(),
    };
    let pattern = data
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let pattern_digest = data
        .get("patternDigest")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let workspace_id = data
        .get("workspaceId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let updated_at = data
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc));
    Some(PermissionRule {
        tool_id,
        scope,
        decision,
        pattern,
        pattern_digest,
        workspace_id,
        updated_at,
    })
}

/// True when `rule` applies to this call.
///
/// A rule with no pattern matches any target; a rule with a pattern needs the
/// target to match the pattern (glob-style). A workspace rule only matches
/// calls bound to the same workspace id.
fn rule_matches(rule: &PermissionRule, ctx: &PermissionContext<'_>) -> bool {
    match rule.scope {
        RuleScope::Session => {
            // A session grant without a workspace (an ad-hoc chat) applies
            // everywhere; one with a workspace is confined to it.
            if let Some(raw) = rule.workspace_id.as_deref() {
                let Some(call_workspace) = ctx.workspace_id else {
                    return false;
                };
                if Uuid::parse_str(raw).ok() != Some(call_workspace) {
                    return false;
                }
            }
        }
        RuleScope::Workspace => {
            let Some(call_workspace) = ctx.workspace_id else {
                return false;
            };
            // Rules with no `workspaceId` (legacy rows or callers that did
            // not pass one through) are matched against the call's workspace:
            // it keeps the rule in effect rather than silently widening it
            // across every workspace.
            let rule_workspace = rule
                .workspace_id
                .as_deref()
                .and_then(|s| Uuid::parse_str(s).ok())
                .unwrap_or(call_workspace);
            if rule_workspace != call_workspace {
                return false;
            }
        }
        RuleScope::Global => {}
    }
    if let Some(digest) = &rule.pattern_digest {
        // A remembered call target is matched by digest so the raw command
        // (which may contain a credential) never has to be persisted.
        let Some(target) = ctx.pattern_target else {
            return false;
        };
        return crate::permission::pattern_digest(target) == *digest;
    }
    if let Some(pattern) = &rule.pattern {
        let Some(target) = ctx.pattern_target else {
            return false;
        };
        match mixd_storage::policy::glob_matches(pattern, target) {
            Some(true) => {}
            Some(false) => return false,
            None => {
                // Fail closed. A Deny rule with a broken pattern still denies
                // (a typo must not open a path); Allow/Ask rules simply do not
                // match, so the caller's safe default applies.
                return rule.decision == PermissionDecision::Deny;
            }
        }
    }
    true
}

// Glob normalisation and matching live in `mixd_storage::policy` so the
// `permission_rules` path and `.mix/policy.toml` evaluate identically. A
// pattern that cannot be parsed returns `None` and the caller fails closed.
impl ToolExecutor {
    pub fn new(storage: Arc<Storage>, secrets: Arc<SecretStore>) -> Self {
        let policy_file = Self::load_policy(&storage);
        Self {
            storage,
            secrets,
            pending: Arc::new(Mutex::new(Vec::new())),
            waiters: Arc::new(Mutex::new(HashMap::new())),
            session_rules: Arc::new(Mutex::new(Vec::new())),
            approval_ttl_secs: Arc::new(AtomicI64::new(DEFAULT_APPROVAL_TTL_SECS)),
            headless_approval: Arc::new(std::sync::Mutex::new(None)),
            policy_file: Arc::new(std::sync::RwLock::new(policy_file)),
            approval_resolve_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Load the workspace policy file only when one actually exists. A missing
    /// file must not turn every `Allow` default into `Ask` purely because the
    /// parser's fail-closed default is `Ask`; once a file is present, failures
    /// in it are fail-closed at evaluation time.
    fn load_policy(storage: &Storage) -> Option<Arc<PolicyFile>> {
        let path = mixd_storage::policy::policy_path(storage.data_dir());
        if !path.is_file() {
            return None;
        }
        Some(Arc::new(mixd_storage::policy::load_policy_file(&path)))
    }

    /// Install (or, with `None`, remove) the policy file used by the permission
    /// path. DB rules always keep priority over it.
    pub fn set_policy_file(&self, policy: Option<Arc<PolicyFile>>) {
        *self
            .policy_file
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = policy;
    }

    /// Fall back to the policy file when no stored rule matched.
    fn policy_or_default(
        &self,
        tool_id: &str,
        ctx: &PermissionContext<'_>,
        default: PermissionDecision,
    ) -> PermissionDecision {
        let guard = self
            .policy_file
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(policy) = guard.as_ref() else {
            return default;
        };
        let decision =
            mixd_storage::policy::evaluate_tool_policy(policy, tool_id, ctx.pattern_target);
        let resolved = match decision {
            FileDecision::Allow => PermissionDecision::Allow,
            FileDecision::Deny => PermissionDecision::Deny,
            FileDecision::Ask => PermissionDecision::Ask,
        };
        // The command-risk gate is not something a policy file may override.
        if resolved == PermissionDecision::Allow
            && tool_id == "terminal"
            && ctx
                .pattern_target
                .is_some_and(|t| crate::risk::classify_command(t).is_dangerous())
        {
            tracing::warn!(tool = %tool_id, "policy file allowed a dangerous terminal command; forcing Ask");
            return PermissionDecision::Ask;
        }
        resolved
    }

    /// Set the non-interactive approval policy used by `mix-agent run`.
    pub fn set_headless_approval(&self, allow: Option<bool>) {
        *self
            .headless_approval
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = allow;
    }

    /// Update the approval prompt lifetime. Values outside the supported range
    /// are clamped rather than rejected: settings written by an older or
    /// newer build must not make every prompt fail.
    pub fn set_approval_ttl_secs(&self, secs: i64) {
        self.approval_ttl_secs.store(
            secs.clamp(MIN_APPROVAL_TTL_SECS, MAX_APPROVAL_TTL_SECS),
            Ordering::Relaxed,
        );
    }

    pub fn approval_ttl_secs(&self) -> i64 {
        self.approval_ttl_secs.load(Ordering::Relaxed)
    }

    /// Number of prompts still waiting for an answer. Used by the tray badge.
    pub async fn pending_count(&self) -> usize {
        self.pending.lock().await.len()
    }

    /// Resolve the effective permission for a Tool.
    ///
    /// Precedence (most specific first, ties broken by `updatedAt` desc, then
    /// by id asc):
    ///
    /// 1. `session` scope + `pattern`
    /// 2. `session` scope, no pattern
    /// 3. `workspace` scope + `pattern`
    /// 4. `workspace` scope, no pattern
    /// 5. `global` scope + `pattern`
    /// 6. `global` scope, no pattern
    ///
    /// Within those buckets the caller is `tool_name`, so two rules on the
    /// same tool compete only when their scope/pattern combination differs.
    /// `Deny` is treated as the safe default: a Deny at any matched bucket
    /// short-circuits an Allow at a less specific bucket, so a workspace
    /// carve-out cannot accidentally widen a global lockdown.
    pub async fn resolve_permission(
        &self,
        tool_id: &str,
        ctx: &PermissionContext<'_>,
        default: PermissionDecision,
    ) -> PermissionDecision {
        let mut rules = {
            let guard = self.session_rules.lock().await;
            guard
                .iter()
                .filter(|rule| rule.tool_id == tool_id)
                .cloned()
                .collect::<Vec<_>>()
        };
        let stored = match self.load_rules(tool_id).await {
            Ok(rules) => rules,
            Err(err) => {
                // Fail-closed: a storage failure must never silently grant a
                // tool call. Even when the caller's `default` is `Allow`, fall
                // back to `Ask` so the user sees the prompt instead of the
                // tool running unchecked.
                tracing::warn!(tool = %tool_id, %err, "loading permission rules failed; failing closed to Ask");
                return PermissionDecision::Ask;
            }
        };
        rules.extend(stored);
        // DB rules above win over `.mix/policy.toml`; the file only supplies
        // the fallback decision when no session/stored rule matched.
        if rules.is_empty() {
            return self.policy_or_default(tool_id, ctx, default);
        }

        let mut allow: Option<&PermissionRule> = None;
        let mut deny: Option<&PermissionRule> = None;
        let mut ask: Option<&PermissionRule> = None;
        // Walk the rules in the order returned by `load_rules` (precedence
        // bucket asc → updated_at desc → id asc) so the first matching rule in
        // each decision bucket wins. We still need to keep all three because a
        // deny at a more specific bucket must beat an allow at a less specific
        // one, and the order in the cursor reflects specificity.
        for rule in &rules {
            if !rule_matches(rule, ctx) {
                continue;
            }
            match rule.decision {
                PermissionDecision::Deny => {
                    if deny.is_none() {
                        deny = Some(rule);
                    }
                }
                PermissionDecision::Allow => {
                    if allow.is_none() {
                        allow = Some(rule);
                    }
                }
                PermissionDecision::Ask => {
                    if ask.is_none() {
                        ask = Some(rule);
                    }
                }
            }
        }

        if let Some(rule) = deny {
            tracing::debug!(
                tool = %tool_id,
                scope = rule.scope.as_str(),
                pattern = rule.pattern.as_deref().unwrap_or(""),
                "permission denied by stored rule"
            );
            return PermissionDecision::Deny;
        }
        if let Some(rule) = allow {
            tracing::debug!(
                tool = %tool_id,
                scope = rule.scope.as_str(),
                pattern = rule.pattern.as_deref().unwrap_or(""),
                "permission granted by stored rule"
            );
            return PermissionDecision::Allow;
        }
        if let Some(rule) = ask {
            tracing::debug!(
                tool = %tool_id,
                scope = rule.scope.as_str(),
                pattern = rule.pattern.as_deref().unwrap_or(""),
                "permission forced to ask by stored rule"
            );
            return PermissionDecision::Ask;
        }
        self.policy_or_default(tool_id, ctx, default)
    }

    /// Read the rules for `tool_id` ordered by precedence bucket, then by
    /// recency. Returned in the same order they would be evaluated.
    async fn load_rules(&self, tool_id: &str) -> anyhow::Result<Vec<PermissionRule>> {
        use sqlx::Row;
        // The `precedence` column is the bucket index (0..=3) computed in SQL
        // so the runtime walks the most specific rules first. Within a bucket
        // the newer rule wins; the id tiebreak keeps older rows whose
        // `updatedAt` is missing deterministic.
        let rows = sqlx::query(
            "SELECT data FROM permission_rules \
             WHERE json_extract(data, '$.toolId') = ?1 \
             ORDER BY CASE \
                 WHEN json_extract(data, '$.scope') = 'workspace' \
                  AND json_extract(data, '$.pattern') IS NOT NULL THEN 0 \
                 WHEN json_extract(data, '$.scope') = 'workspace' THEN 1 \
                 WHEN json_extract(data, '$.scope') = 'global' \
                  AND json_extract(data, '$.pattern') IS NOT NULL THEN 2 \
                 ELSE 3 \
             END ASC, \
             COALESCE(json_extract(data, '$.updatedAt'), '') DESC, \
             id ASC",
        )
        .bind(tool_id)
        .fetch_all(self.storage.pool())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let raw: String = row.try_get("data").unwrap_or_default();
            let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            let Some(rule) = parse_rule(&parsed) else {
                continue;
            };
            out.push(rule);
        }
        Ok(out)
    }

    /// Persist the outcome of an approval prompt and wake the paused Run.
    ///
    /// `tool_id` is the Tool the rule must be keyed on — using the approval id
    /// here would create a rule that can never match a later call.
    /// Resolve an approval from a trusted caller (the desktop UI and tests).
    ///
    /// The prompt must still be pending; a second resolution or one that races
    /// the expiry reaper is an error rather than a silent double grant.
    pub async fn resolve_approval(
        &self,
        approval_id: &str,
        tool_id: &str,
        outcome: ApprovalOutcome,
    ) -> anyhow::Result<()> {
        let _guard = self.approval_resolve_lock.lock().await;
        let dangerous = self
            .pending
            .lock()
            .await
            .iter()
            .find(|prompt| prompt.approval_id == approval_id)
            .and_then(|prompt| prompt.risk.as_ref())
            .is_some_and(|risk| risk.is_dangerous());
        if !self
            .pending
            .lock()
            .await
            .iter()
            .any(|prompt| prompt.approval_id == approval_id)
        {
            anyhow::bail!("unknown or expired approval {approval_id}");
        }
        self.resolve_approval_inner(approval_id, tool_id, outcome, dangerous)
            .await
    }

    /// Resolve an approval from an untrusted caller (the HTTP bridge).
    ///
    /// Every guard — pending state, risk acknowledgement, remember rules — is
    /// checked under one lock so a concurrent decision cannot slip between the
    /// validation and the mutation.
    pub async fn resolve_approval_checked(
        &self,
        approval_id: &str,
        tool_id: &str,
        outcome: ApprovalOutcome,
        acknowledge_risk: bool,
    ) -> anyhow::Result<()> {
        let _guard = self.approval_resolve_lock.lock().await;
        let pending = self.pending.lock().await;
        let prompt = pending
            .iter()
            .find(|prompt| prompt.approval_id == approval_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown or expired approval {approval_id}"))?;
        drop(pending);

        let approved = matches!(outcome, ApprovalOutcome::Approve { .. });
        let (remember, remember_session) = match outcome {
            ApprovalOutcome::Approve {
                remember,
                remember_session,
            } => (remember, remember_session),
            ApprovalOutcome::Deny => (false, false),
        };
        let dangerous = prompt.risk.as_ref().is_some_and(|risk| risk.is_dangerous());
        if dangerous && approved {
            if remember || remember_session {
                anyhow::bail!(
                    "dangerous commands cannot be remembered; approve this one call explicitly"
                );
            }
            if !acknowledge_risk {
                anyhow::bail!("dangerous command requires explicit acknowledgement");
            }
        }
        if remember && !approved {
            anyhow::bail!("remember requires an approval grant");
        }
        self.resolve_approval_inner(approval_id, tool_id, outcome, dangerous)
            .await
    }

    async fn resolve_approval_inner(
        &self,
        approval_id: &str,
        tool_id: &str,
        outcome: ApprovalOutcome,
        dangerous: bool,
    ) -> anyhow::Result<()> {
        let decision = match outcome {
            ApprovalOutcome::Approve { .. } => PermissionDecision::Allow,
            ApprovalOutcome::Deny => PermissionDecision::Deny,
        };
        let mut outcome = outcome;
        let mut remember = matches!(outcome, ApprovalOutcome::Approve { remember: true, .. });
        let mut remember_session = matches!(
            outcome,
            ApprovalOutcome::Approve {
                remember_session: true,
                ..
            }
        );
        let approved = matches!(outcome, ApprovalOutcome::Approve { .. });
        let (workspace_id, pattern, pattern_digest) = self.approval_scope(approval_id).await;
        // Defense in depth: the app refuses "Always Allow" for a dangerous
        // command, but a different client (CLI, HTTP API, a future UI) could
        // still send `remember: true`. Use the risk graded when the prompt was
        // opened — the stored target is redacted now, so re-classifying it
        // here would miss exactly the commands we need to protect.
        // Also re-grade the redacted target: callers using `request_approval`
        // (rather than the graded variant) still carry a safe risk object, and
        // redaction normally leaves the command itself intact.
        let dangerous = dangerous
            || (tool_id == "terminal"
                && pattern
                    .as_deref()
                    .is_some_and(|target| crate::risk::classify_command(target).is_dangerous()));
        if dangerous && (remember || remember_session) {
            tracing::warn!(
                tool = %tool_id,
                target = pattern.as_deref().unwrap_or(""),
                "refusing to persist an approval rule for a dangerous terminal command"
            );
            remember = false;
            remember_session = false;
            if approved {
                outcome = ApprovalOutcome::Approve {
                    remember: false,
                    remember_session: false,
                };
            }
        }
        if remember {
            let scope = if workspace_id.is_some() {
                RuleScope::Workspace
            } else {
                RuleScope::Global
            };
            self.remember_rule_resolved(
                tool_id,
                decision,
                scope,
                workspace_id,
                pattern.as_deref(),
                pattern_digest.as_deref(),
            )
            .await?;
        }
        if remember_session {
            self.remember_session_rule(
                tool_id,
                decision,
                workspace_id,
                pattern.as_deref(),
                pattern_digest.as_deref(),
            )
            .await;
        }
        sqlx::query(
            "UPDATE approvals SET status = ?1, data = json_set(data, '$.grantedAt', ?2) \
             WHERE id = ?3",
        )
        .bind(match outcome {
            ApprovalOutcome::Approve { .. } => "approved",
            ApprovalOutcome::Deny => "denied",
        })
        .bind(Utc::now().to_rfc3339())
        .bind(approval_id)
        .execute(self.storage.pool())
        .await?;

        self.pending
            .lock()
            .await
            .retain(|p| p.approval_id != approval_id);
        // Records the end of the wait as well as its outcome: a prompt that is
        // opened but never seen closing again is how a leaked waiter shows up.
        tracing::info!(approval = %approval_id, tool = %tool_id, approved, "approval resolved");
        if let Some(waiter) = self.waiters.lock().await.remove(approval_id) {
            let _ = waiter.tx.send(outcome);
        }
        // Audit the decision (A5, best-effort). The persisted approvals row
        // still carries the run / tool-call ids and the prompt arguments, so
        // read the context from there (the in-memory prompt is already gone:
        // it was retained above). Redaction happens inside `append_audit`.
        let (audit_run, audit_call, persisted_args) =
            self.approval_audit_context(approval_id).await;
        let run_uuid = Uuid::parse_str(&audit_run).unwrap_or_else(|_| Uuid::nil());
        let decision = match decision {
            PermissionDecision::Allow => "approved",
            PermissionDecision::Deny => "denied_by_user",
            PermissionDecision::Ask => "ask",
        };
        self.append_audit(
            run_uuid,
            &audit_call,
            Some(approval_id),
            tool_id,
            &persisted_args,
            decision,
        )
        .await;
        Ok(())
    }

    /// The (run_id, tool_call_id, arguments) a prompt was opened for, read
    /// from the persisted `approvals` row.
    async fn approval_audit_context(
        &self,
        approval_id: &str,
    ) -> (String, String, serde_json::Value) {
        use sqlx::Row;
        let row = sqlx::query("SELECT run_id, tool_call_id, data FROM approvals WHERE id = ?1")
            .bind(approval_id)
            .fetch_optional(self.storage.pool())
            .await
            .ok()
            .flatten();
        let Some(row) = row else {
            return (
                Uuid::nil().to_string(),
                String::new(),
                serde_json::json!({}),
            );
        };
        let run_id: String = row.try_get("run_id").unwrap_or_default();
        let tool_call_id: String = row.try_get("tool_call_id").unwrap_or_default();
        let raw: String = row.try_get("data").unwrap_or_default();
        let data: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let args = data.get("arguments").cloned().unwrap_or_default();
        (run_id, tool_call_id, args)
    }

    /// Write a durable permission rule for a Tool ("Always Allow" / "Never").
    ///
    /// The rule is scoped to the call that prompted it: the run's workspace and
    /// the tool's pattern target (a path, command, URL, browser action, or MCP
    /// connection id). Remembering a patternless *global* allow instead — which
    /// is what this used to do — meant one click on a single `terminal`
    /// command silently allowed every future `terminal` call, including
    /// destructive ones, plus every future `files` delete and `browser`
    /// `evaluate`. `scope` is normalised: a workspace rule without a workspace
    /// id downgrades to global rather than becoming unmatchable.
    pub async fn remember_rule(
        &self,
        tool_id: &str,
        decision: PermissionDecision,
        scope: RuleScope,
        workspace_id: Option<Uuid>,
        pattern: Option<&str>,
    ) -> anyhow::Result<()> {
        // Callers of this public API pass glob patterns (Settings, tests).
        // Only approval-driven rules go through `remember_rule_resolved` with
        // a digest, so glob semantics stay intact here.
        self.remember_rule_resolved(tool_id, decision, scope, workspace_id, pattern, None)
            .await
    }

    /// Persist a rule whose target has already been redacted and digested by
    /// [`Self::approval_scope`]. The raw command must never reach SQLite: a
    /// single "Always Allow" on `curl -H 'X-Key: …'` used to write the key into
    /// `permission_rules.data.pattern` and into the rule id permanently.
    #[allow(clippy::too_many_arguments)]
    async fn remember_rule_resolved(
        &self,
        tool_id: &str,
        decision: PermissionDecision,
        scope: RuleScope,
        workspace_id: Option<Uuid>,
        pattern: Option<&str>,
        pattern_digest: Option<&str>,
    ) -> anyhow::Result<()> {
        let (scope, workspace_id) = match (scope, workspace_id) {
            (RuleScope::Workspace, Some(ws)) => (RuleScope::Workspace, Some(ws)),
            _ => (RuleScope::Global, None),
        };
        let pattern = pattern.map(str::trim).filter(|p| !p.is_empty());
        let digest = pattern_digest.filter(|d| !d.is_empty());
        let rule = json!({
            "toolId": tool_id,
            "scope": scope.as_str(),
            "decision": match decision {
                PermissionDecision::Allow => "allow",
                PermissionDecision::Ask => "ask",
                PermissionDecision::Deny => "deny",
            },
            // Display only. The raw target is matched through its digest.
            "pattern": pattern,
            "patternDigest": digest,
            "workspaceId": workspace_id.map(|w| w.to_string()),
            "updatedAt": Utc::now().to_rfc3339(),
        });
        // Deterministic id so repeating "Always Allow" for the same target
        // refreshes one row instead of accumulating duplicates. Use the digest
        // rather than the command text so the id cannot carry a credential.
        // The `prompt:` prefix keeps these apart from the rule ids
        // `set_permission_rule` writes, so a Settings edit is never clobbered.
        let rule_id = match (workspace_id, digest) {
            (Some(ws), Some(d)) => format!("prompt:{tool_id}:{ws}:{d}"),
            (Some(ws), None) => format!("prompt:{tool_id}:{ws}"),
            (None, Some(d)) => format!("prompt:{tool_id}::{d}"),
            (None, None) => format!("prompt:{tool_id}"),
        };
        sqlx::query(
            "INSERT INTO permission_rules (id, owner_id, data) VALUES (?1, ?2, ?3) \
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
        )
        .bind(rule_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(rule.to_string())
        .execute(self.storage.pool())
        .await?;
        Ok(())
    }

    /// Record an in-memory allow/deny for the rest of the process.
    ///
    /// Scoped to the tool, the workspace and the same pattern target as a
    /// durable rule, but never written to SQLite: closing the app forgets it.
    #[allow(clippy::too_many_arguments)]
    async fn remember_session_rule(
        &self,
        tool_id: &str,
        decision: PermissionDecision,
        workspace_id: Option<Uuid>,
        pattern: Option<&str>,
        pattern_digest: Option<&str>,
    ) {
        let pattern = pattern.map(str::trim).filter(|p| !p.is_empty());
        let digest = pattern_digest.filter(|d| !d.is_empty());
        let mut rules = self.session_rules.lock().await;
        rules.retain(|rule| {
            !(rule.tool_id == tool_id
                && rule.workspace_id == workspace_id.map(|id| id.to_string())
                && rule.pattern.as_deref() == pattern
                && rule.pattern_digest.as_deref() == digest)
        });
        rules.push(PermissionRule {
            tool_id: tool_id.to_string(),
            scope: RuleScope::Session,
            decision,
            pattern: pattern.map(str::to_string),
            pattern_digest: digest.map(str::to_string),
            workspace_id: workspace_id.map(|id| id.to_string()),
            updated_at: Some(Utc::now()),
        });
    }

    /// The workspace, redacted pattern target and target digest a prompt was
    /// opened for, read from the persisted `approvals` row.
    ///
    /// This is what lets "Always Allow" remember the *call* rather than the
    /// tool: the in-memory prompt may already be gone by the time the user
    /// answers, but the row still carries the scope the engine captured.
    async fn approval_scope(
        &self,
        approval_id: &str,
    ) -> (Option<Uuid>, Option<String>, Option<String>) {
        use sqlx::Row;
        let row = sqlx::query("SELECT data FROM approvals WHERE id = ?")
            .bind(approval_id)
            .fetch_optional(self.storage.pool())
            .await
            .ok()
            .flatten();
        let Some(row) = row else {
            return (None, None, None);
        };
        let raw: String = row.try_get("data").unwrap_or_default();
        let data: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let workspace_id = data
            .get("workspaceId")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        let pattern = data
            .get("patternTarget")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let digest = data
            .get("patternDigest")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        (workspace_id, pattern, digest)
    }

    /// Which Tool a pending prompt belongs to, so a "remember" decision can be
    /// written against the right permission rule. Falls back to the persisted
    /// row when the in-memory prompt is already gone.
    pub async fn tool_name_for_approval(&self, approval_id: &str) -> Option<String> {
        if let Some(p) = self
            .pending
            .lock()
            .await
            .iter()
            .find(|p| p.approval_id == approval_id)
        {
            return Some(p.tool_name.clone());
        }
        use sqlx::Row;
        let row = sqlx::query("SELECT data FROM approvals WHERE id = ?")
            .bind(approval_id)
            .fetch_optional(self.storage.pool())
            .await
            .ok()??;
        let raw: String = row.try_get("data").ok()?;
        let data: serde_json::Value = serde_json::from_str(&raw).ok()?;
        data.get("toolName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// How many wake-up channels are still held for unanswered prompts.
    ///
    /// A non-zero value after every prompt has been resolved or expired means
    /// a `waiters` leak: each entry pins an engine task's `rx` and grows the
    /// map for the lifetime of the process.
    pub async fn open_waiters(&self) -> usize {
        self.waiters.lock().await.len()
    }

    /// The prompts the UI should render right now, dropping expired ones.
    pub async fn list_pending(&self) -> Vec<PendingApproval> {
        // Sweep first so an expired prompt is neither rendered nor left behind
        // as a `pending` row when nobody polls again.
        self.sweep_expired().await;
        self.pending.lock().await.clone()
    }

    /// The current deadline for a prompt, or `None` when it is no longer
    /// pending. The Run Engine re-reads this after its first deadline fires so
    /// an extension granted while it slept does not turn into a denial.
    pub async fn approval_expiry(&self, approval_id: &str) -> Option<chrono::DateTime<Utc>> {
        self.pending
            .lock()
            .await
            .iter()
            .find(|prompt| prompt.approval_id == approval_id)
            .map(|prompt| prompt.expires_at)
    }

    /// Push a pending prompt's deadline out by `seconds`.
    ///
    /// The in-memory prompt, the parked Run's waiter and the persisted
    /// `approvals.expires` row move together; otherwise the UI would show a new
    /// deadline while the expiry reaper (or a restarted process) still used the
    /// old one. Extension is serialised with resolution so an "extend" cannot
    /// resurrect a prompt the user just answered.
    pub async fn extend_approval(
        &self,
        approval_id: &str,
        seconds: i64,
    ) -> anyhow::Result<(Uuid, String, chrono::DateTime<Utc>)> {
        if !(MIN_APPROVAL_EXTENSION_SECS..=MAX_APPROVAL_EXTENSION_SECS).contains(&seconds) {
            anyhow::bail!(
                "approval extension must be between {MIN_APPROVAL_EXTENSION_SECS} and \
                 {MAX_APPROVAL_EXTENSION_SECS} seconds"
            );
        }
        let _guard = self.approval_resolve_lock.lock().await;
        let now = Utc::now();
        let (run_id, tool_call_id, new_expires) = {
            let pending = self.pending.lock().await;
            let prompt = pending
                .iter()
                .find(|prompt| prompt.approval_id == approval_id)
                .ok_or_else(|| anyhow::anyhow!("unknown or expired approval {approval_id}"))?;
            let base = prompt.expires_at.max(now);
            let cap = now + chrono::Duration::seconds(MAX_APPROVAL_TTL_SECS);
            let next = (base + chrono::Duration::seconds(seconds)).min(cap);
            (prompt.run_id, prompt.tool_call_id.clone(), next)
        };
        // Persist first, then move the in-memory copies. Reversing this order
        // left the executor and UI believing an extension had landed when the
        // database write failed, and a restart silently reverted it.
        let updated =
            sqlx::query("UPDATE approvals SET expires = ?1 WHERE id = ?2 AND status = 'pending'")
                .bind(new_expires.to_rfc3339())
                .bind(approval_id)
                .execute(self.storage.pool())
                .await?;
        if updated.rows_affected() == 0 {
            anyhow::bail!("approval {approval_id} is no longer pending");
        }
        {
            let mut pending = self.pending.lock().await;
            if let Some(prompt) = pending
                .iter_mut()
                .find(|prompt| prompt.approval_id == approval_id)
            {
                prompt.expires_at = new_expires;
            }
        }
        {
            let mut waiters = self.waiters.lock().await;
            if let Some(waiter) = waiters.get_mut(approval_id) {
                waiter.expires_at = new_expires;
            }
        }
        tracing::info!(
            approval = %approval_id,
            expires_at = %new_expires,
            "approval deadline extended"
        );
        Ok((run_id, tool_call_id, new_expires))
    }

    /// Reclaim prompts whose TTL has passed.
    ///
    /// Three things are leaked by an unanswered prompt and all three are
    /// released here: the in-memory entry shown by the UI, the `waiters`
    /// channel the engine is (or was) parked on, and the `approvals` row, which
    /// stays `pending` forever because the engine's timeout branch never calls
    /// [`ToolExecutor::resolve_approval`]. Idempotent and safe to call from
    /// both the UI poll path and the per-prompt deadline task. Returns the ids
    /// that were actually closed.
    pub async fn sweep_expired(&self) -> Vec<String> {
        // Same lock as `resolve_approval`: a prompt must not be reclaimed by
        // expiry in the window between a caller's "still pending?" check and
        // its state/rule mutations.
        let _guard = self.approval_resolve_lock.lock().await;
        let now = Utc::now();
        let mut expired: Vec<String> = {
            let mut guard = self.pending.lock().await;
            let ids = guard
                .iter()
                .filter(|p| p.expires_at <= now)
                .map(|p| p.approval_id.clone())
                .collect();
            guard.retain(|p| p.expires_at > now);
            ids
        };
        {
            let mut waiters = self.waiters.lock().await;
            // The channel's own deadline is checked as well: a prompt that was
            // removed from `pending` without being resolved (a cancelled Run,
            // a UI that dropped it) would otherwise pin this map forever.
            let ids: Vec<String> = waiters
                .iter()
                .filter(|(id, w)| w.expires_at <= now || expired.contains(*id))
                .map(|(id, _)| id.clone())
                .collect();
            for id in &ids {
                waiters.remove(id);
            }
            for id in ids {
                if !expired.contains(&id) {
                    expired.push(id);
                }
            }
        }
        if expired.is_empty() {
            return expired;
        }
        for id in &expired {
            // `status = 'pending'` keeps a decision made in the same instant
            // from being overwritten by the automatic expiry.
            let result = sqlx::query(
                "UPDATE approvals SET status = 'expired' WHERE id = ?1 AND status = 'pending'",
            )
            .bind(id)
            .execute(self.storage.pool())
            .await;
            if let Err(err) = result {
                tracing::warn!(approval = %id, %err, "closing an expired approval row failed");
            }
        }
        tracing::warn!(
            count = expired.len(),
            "approval prompts expired unanswered; waiters and pending rows reclaimed"
        );
        // Audit each expiry (A5, best-effort) so an unanswered prompt is still
        // visible in the trail. The tool name comes from the in-memory prompt
        // when it is still there, else from the persisted row.
        for id in &expired {
            let (run_id, tool_call_id, args, tool_name) = self.expired_audit_context(id).await;
            let run_uuid = Uuid::parse_str(&run_id).unwrap_or_else(|_| Uuid::nil());
            self.append_audit(
                run_uuid,
                &tool_call_id,
                Some(id),
                &tool_name,
                &args,
                "expired",
            )
            .await;
        }
        expired
    }

    /// Context for an expired prompt: (run_id, tool_call_id, arguments, tool_name).
    async fn expired_audit_context(
        &self,
        approval_id: &str,
    ) -> (String, String, serde_json::Value, String) {
        use sqlx::Row;
        if let Some(p) = self
            .pending
            .lock()
            .await
            .iter()
            .find(|p| p.approval_id == approval_id)
        {
            return (
                p.run_id.to_string(),
                p.tool_call_id.clone(),
                p.arguments.clone(),
                p.tool_name.clone(),
            );
        }
        let row = sqlx::query("SELECT run_id, tool_call_id, data FROM approvals WHERE id = ?1")
            .bind(approval_id)
            .fetch_optional(self.storage.pool())
            .await
            .ok()
            .flatten();
        let Some(row) = row else {
            return (
                Uuid::nil().to_string(),
                String::new(),
                serde_json::json!({}),
                String::new(),
            );
        };
        let run_id: String = row.try_get("run_id").unwrap_or_default();
        let tool_call_id: String = row.try_get("tool_call_id").unwrap_or_default();
        let raw: String = row.try_get("data").unwrap_or_default();
        let data: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let args = data.get("arguments").cloned().unwrap_or_default();
        let tool = data
            .get("toolName")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        (run_id, tool_call_id, args, tool)
    }

    /// Pause the Run on a Tool call until the user answers the prompt.
    ///
    /// Returns the receiver that yields the outcome plus the prompt's expiry so
    /// the caller can bound its own wait. `None` if the prompt could not be
    /// persisted.
    pub async fn request_approval(
        &self,
        run_id: Uuid,
        tool_call_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        workspace_id: Option<Uuid>,
        pattern_target: Option<String>,
    ) -> anyhow::Result<(oneshot::Receiver<ApprovalOutcome>, chrono::DateTime<Utc>)> {
        self.request_approval_graded(
            run_id,
            tool_call_id,
            tool_name,
            arguments,
            workspace_id,
            pattern_target,
            crate::risk::RiskAssessment::safe(),
        )
        .await
    }

    /// Same as [`Self::request_approval`], but records the command-level risk
    /// the caller computed so the prompt can show why it is different.
    #[allow(clippy::too_many_arguments)]
    pub async fn request_approval_graded(
        &self,
        run_id: Uuid,
        tool_call_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        workspace_id: Option<Uuid>,
        pattern_target: Option<String>,
        risk: crate::risk::RiskAssessment,
    ) -> anyhow::Result<(oneshot::Receiver<ApprovalOutcome>, chrono::DateTime<Utc>)> {
        // A headless run has no card to answer. Resolve before touching the
        // database so the run does not create an approvals row nobody can see.
        let headless = *self
            .headless_approval
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(allow) = headless {
            if risk.is_dangerous() && allow {
                tracing::warn!(
                    "headless --yes auto-approved a dangerous command; prefer an explicit permission rule"
                );
            }
            let (tx, rx) = oneshot::channel();
            let outcome = if allow {
                ApprovalOutcome::Approve {
                    remember: false,
                    remember_session: false,
                }
            } else {
                ApprovalOutcome::Deny
            };
            let _ = tx.send(outcome);
            // Use the configured TTL even though this prompt is resolved
            // immediately: callers show a countdown/deadline from this value,
            // and a `set_approval_ttl_secs` change must be reflected here too.
            return Ok((
                rx,
                Utc::now() + chrono::Duration::seconds(self.approval_ttl_secs()),
            ));
        }
        let now = Utc::now();
        let ttl_secs = self.approval_ttl_secs();
        let approval = PendingApproval {
            approval_id: Uuid::new_v4().to_string(),
            run_id,
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments,
            created_at: now,
            expires_at: now + chrono::Duration::seconds(ttl_secs),
            remember: false,
            risk: Some(risk.clone()),
        };
        self.record_approval(
            &approval.approval_id,
            run_id,
            tool_call_id,
            tool_name,
            approval.arguments.clone(),
            approval.expires_at,
            workspace_id,
            pattern_target,
            risk,
        )
        .await?;

        let (tx, rx) = oneshot::channel();
        self.waiters.lock().await.insert(
            approval.approval_id.clone(),
            Waiter {
                tx,
                expires_at: approval.expires_at,
            },
        );
        self.pending.lock().await.push(approval.clone());
        // A Run is parked on this prompt now; if nobody answers, the TTL sweep
        // below is the only thing that will ever move it again.
        tracing::info!(
            approval = %approval.approval_id,
            run = %run_id,
            tool = %tool_name,
            expires_in_secs = ttl_secs,
            "run is waiting for a tool approval"
        );

        // The engine abandons the prompt at the TTL without resolving it, so a
        // deadline task is what actually closes the row and drops the channel.
        // Without it a Run that is cancelled or left hanging keeps a `pending`
        // approvals row and a `waiters` entry for the lifetime of the process.
        let sweeper = self.clone();
        let sweep_id = approval.approval_id.clone();
        let tool = approval.tool_name.clone();
        tokio::spawn(async move {
            // Loop instead of sleeping once: an extension moves the deadline
            // while this task is parked, and a single sleep would either
            // expire the prompt early or never revisit the new deadline.
            loop {
                let Some(expiry) = sweeper.approval_expiry(&sweep_id).await else {
                    return;
                };
                let wait = (expiry - Utc::now())
                    .to_std()
                    .unwrap_or(std::time::Duration::ZERO);
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
                if sweeper
                    .sweep_expired()
                    .await
                    .iter()
                    .any(|id| id == &sweep_id)
                {
                    tracing::warn!(
                        approval = %sweep_id,
                        tool = %tool,
                        "approval TTL elapsed before the user answered"
                    );
                    return;
                }
                if sweeper.approval_expiry(&sweep_id).await.is_none() {
                    return;
                }
                // `wait` was zero but the prompt survived the sweep (clock
                // skew or an extension landing in the same tick). Yield rather
                // than spin the runtime.
                if wait.is_zero() {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        });

        Ok((rx, approval.expires_at))
    }

    /// Persist a Tool call record (executing / succeeded / failed) so the UI
    /// can render the timeline.
    ///
    /// Also appends one row to the append-only `audit_log` (A5, best-effort):
    /// the audit arguments are the redacted display copy so secrets never
    /// reach the log in plaintext. An audit failure only warns — it never
    /// fails the Tool call.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_tool_call(
        &self,
        run_id: Uuid,
        tool_call_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        status: &str,
        result: Option<ToolResult>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = Utc::now();
        let started = now.to_rfc3339();
        let data = json!({
            "toolName": tool_name,
            "arguments": arguments,
            "result": result.as_ref().map(|r| &r.content),
            "error": error,
            "startedAt": started,
            "finishedAt": now.to_rfc3339(),
        });
        sqlx::query(
            "INSERT INTO tool_calls (id, owner_id, data, run_id, status) VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET data = excluded.data, status = excluded.status",
        )
        .bind(tool_call_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(run_id.to_string())
        .bind(status)
        .execute(self.storage.pool())
        .await?;
        self.append_audit(run_id, tool_call_id, None, tool_name, &arguments, status)
            .await;
        Ok(())
    }

    /// Best-effort audit append for a Tool call lifecycle event.
    ///
    /// Looks up the approval row for `tool_call_id` (if any) so the audit row
    /// carries the link without the caller having to thread it through.
    /// Failures only warn; the `?` in the caller above has already persisted
    /// the `tool_calls` row, and a stuck Run is worse than a missing audit row.
    async fn append_audit(
        &self,
        run_id: Uuid,
        tool_call_id: &str,
        approval_id: Option<&str>,
        tool_name: &str,
        arguments: &serde_json::Value,
        decision: &str,
    ) {
        // Redact before persisting: one `cat .env` must not copy a live key
        // into the audit trail.
        let mut audit_args = arguments.clone();
        crate::redact::redact_arguments(&mut audit_args);
        let approval_id_owned: Option<String> = if let Some(id) = approval_id {
            Some(id.to_string())
        } else {
            sqlx::query_scalar::<_, String>(
                "SELECT id FROM approvals WHERE tool_call_id = ?1 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(tool_call_id)
            .fetch_optional(self.storage.pool())
            .await
            .unwrap_or(None)
        };
        if let Err(err) = mixd_storage::audit::append_entry(
            self.storage.pool(),
            mixd_storage::records::SINGLE_USER_ID,
            &run_id.to_string(),
            tool_call_id,
            approval_id_owned.as_deref(),
            tool_name,
            &audit_args,
            decision,
        )
        .await
        {
            tracing::warn!(tool = %tool_name, status = %decision, %err, "audit append failed; continuing");
        }
    }

    /// Persist an approval row.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub async fn record_approval(
        &self,
        approval_id: &str,
        run_id: Uuid,
        tool_call_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        expires: chrono::DateTime<Utc>,
        workspace_id: Option<Uuid>,
        pattern_target: Option<String>,
        risk: crate::risk::RiskAssessment,
    ) -> anyhow::Result<()> {
        // Keep only a redacted target and a digest on disk. The raw command may
        // contain a credential and this row survives the run.
        let pattern_digest = pattern_target
            .as_deref()
            .map(crate::permission::pattern_digest);
        let pattern_target = pattern_target
            .as_deref()
            .map(crate::redact::redact_text)
            .filter(|s| !s.is_empty());
        let data = json!({
            "toolName": tool_name,
            "arguments": arguments,
            "grantedAt": null,
            "remember": false,
            // Captured at prompt time so "Always Allow" can scope the rule to
            // this call instead of the whole tool.
            "workspaceId": workspace_id.map(|w| w.to_string()),
            "patternTarget": pattern_target,
            "patternDigest": pattern_digest,
            // Persisted for the audit trail: a dangerous command that was
            // approved should still be identifiable after the run finishes.
            "risk": risk,
        });
        sqlx::query(
            "INSERT INTO approvals (id, owner_id, data, run_id, tool_call_id, status, expires) \
             VALUES (?, ?, ?, ?, ?, 'pending', ?)",
        )
        .bind(approval_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(run_id.to_string())
        .bind(tool_call_id)
        .bind(expires.to_rfc3339())
        .execute(self.storage.pool())
        .await?;
        Ok(())
    }
}
