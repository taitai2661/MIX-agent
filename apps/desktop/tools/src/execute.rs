//! Serialised executor for side-effecting Tool calls: the approval queue, the
//! permission-rule resolver, and the persistent approval storage.

use std::collections::HashMap;
use std::sync::Arc;

use crate::approval::{ApprovalOutcome, PendingApproval};
use crate::permission::{PermissionDecision, PermissionRule, RuleScope};
use crate::ToolResult;
use chrono::Utc;
use glob::Pattern;
use mixd_storage::{SecretStore, Storage};
use serde_json::json;
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

/// How long an approval prompt stays answerable before the Run resumes and the
/// Tool is treated as denied.
const APPROVAL_TTL_SECS: i64 = 300;

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
    if let Some(pattern) = &rule.pattern {
        let Some(target) = ctx.pattern_target else {
            return false;
        };
        if !pattern_matches(pattern, target) {
            return false;
        }
    }
    true
}

fn pattern_matches(pattern: &str, target: &str) -> bool {
    // `glob::Pattern::new` returns Err for things that contain a backslash or
    // a bracket; a plain substring check keeps those users covered.
    match Pattern::new(pattern) {
        Ok(matcher) => matcher.matches(target),
        Err(_) => target.contains(pattern),
    }
}
impl ToolExecutor {
    pub fn new(storage: Arc<Storage>, secrets: Arc<SecretStore>) -> Self {
        Self {
            storage,
            secrets,
            pending: Arc::new(Mutex::new(Vec::new())),
            waiters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Resolve the effective permission for a Tool.
    ///
    /// Precedence (most specific first, ties broken by `updatedAt` desc, then
    /// by id asc):
    ///
    /// 1. `workspace` scope + `pattern`
    /// 2. `workspace` scope, no pattern
    /// 3. `global` scope + `pattern`
    /// 4. `global` scope, no pattern
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
        let rules = match self.load_rules(tool_id).await {
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
        if rules.is_empty() {
            return default;
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
        default
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
    pub async fn resolve_approval(
        &self,
        approval_id: &str,
        tool_id: &str,
        outcome: ApprovalOutcome,
    ) -> anyhow::Result<()> {
        let decision = match outcome {
            ApprovalOutcome::Approve { .. } => PermissionDecision::Allow,
            ApprovalOutcome::Deny => PermissionDecision::Deny,
        };
        let remember = matches!(outcome, ApprovalOutcome::Approve { remember: true });
        let approved = matches!(outcome, ApprovalOutcome::Approve { .. });
        if remember {
            let (workspace_id, pattern) = self.approval_scope(approval_id).await;
            let scope = if workspace_id.is_some() {
                RuleScope::Workspace
            } else {
                RuleScope::Global
            };
            self.remember_rule(tool_id, decision, scope, workspace_id, pattern.as_deref())
                .await?;
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
        Ok(())
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
        let (scope, workspace_id) = match (scope, workspace_id) {
            (RuleScope::Workspace, Some(ws)) => (RuleScope::Workspace, Some(ws)),
            _ => (RuleScope::Global, None),
        };
        let pattern = pattern.map(str::trim).filter(|p| !p.is_empty());
        let rule = json!({
            "toolId": tool_id,
            "scope": scope.as_str(),
            "decision": match decision {
                PermissionDecision::Allow => "allow",
                PermissionDecision::Ask => "ask",
                PermissionDecision::Deny => "deny",
            },
            "pattern": pattern,
            "workspaceId": workspace_id.map(|w| w.to_string()),
            "updatedAt": Utc::now().to_rfc3339(),
        });
        // Deterministic id so repeating "Always Allow" for the same target
        // refreshes one row instead of accumulating duplicates. The `prompt:`
        // prefix keeps these apart from the rule ids `set_permission_rule`
        // writes, so a later Settings edit is never clobbered.
        let rule_id = match (workspace_id, pattern) {
            (Some(ws), Some(p)) => format!("prompt:{tool_id}:{ws}:{p}"),
            (Some(ws), None) => format!("prompt:{tool_id}:{ws}"),
            (None, Some(p)) => format!("prompt:{tool_id}::{p}"),
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

    /// The workspace and pattern target a prompt was opened for, read from the
    /// persisted `approvals` row.
    ///
    /// This is what lets "Always Allow" remember the *call* rather than the
    /// tool: the in-memory prompt may already be gone by the time the user
    /// answers, but the row still carries the scope the engine captured.
    async fn approval_scope(&self, approval_id: &str) -> (Option<Uuid>, Option<String>) {
        use sqlx::Row;
        let row = sqlx::query("SELECT data FROM approvals WHERE id = ?")
            .bind(approval_id)
            .fetch_optional(self.storage.pool())
            .await
            .ok()
            .flatten();
        let Some(row) = row else {
            return (None, None);
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
        (workspace_id, pattern)
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
        expired
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
        let now = Utc::now();
        let approval = PendingApproval {
            approval_id: Uuid::new_v4().to_string(),
            run_id,
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments,
            created_at: now,
            expires_at: now + chrono::Duration::seconds(APPROVAL_TTL_SECS),
            remember: false,
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
            expires_in_secs = APPROVAL_TTL_SECS,
            "run is waiting for a tool approval"
        );

        // The engine abandons the prompt at the TTL without resolving it, so a
        // deadline task is what actually closes the row and drops the channel.
        // Without it a Run that is cancelled or left hanging keeps a `pending`
        // approvals row and a `waiters` entry for the lifetime of the process.
        let sweeper = self.clone();
        let sweep_id = approval.approval_id.clone();
        let tool = approval.tool_name.clone();
        let wait = (approval.expires_at - now)
            .to_std()
            .unwrap_or(std::time::Duration::ZERO);
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
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
            }
        });

        Ok((rx, approval.expires_at))
    }

    /// Persist a Tool call record (executing / succeeded / failed) so the UI
    /// can render the timeline.
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
        Ok(())
    }

    /// Persist an approval row.
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
    ) -> anyhow::Result<()> {
        let data = json!({
            "toolName": tool_name,
            "arguments": arguments,
            "grantedAt": null,
            "remember": false,
            // Captured at prompt time so "Always Allow" can scope the rule to
            // this call instead of the whole tool.
            "workspaceId": workspace_id.map(|w| w.to_string()),
            "patternTarget": pattern_target,
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
