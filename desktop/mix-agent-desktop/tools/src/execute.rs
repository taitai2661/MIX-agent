//! Serialised executor for side-effecting Tool calls: the approval queue, the
//! permission-rule resolver, and the persistent approval storage.

use std::collections::HashMap;
use std::sync::Arc;

use crate::approval::{ApprovalOutcome, PendingApproval};
use crate::permission::PermissionDecision;
use crate::ToolResult;
use chrono::Utc;
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
    /// Precedence: the narrowest matching stored rule wins, otherwise the
    /// Tool's own default applies.
    pub async fn resolve_permission(
        &self,
        tool_id: &str,
        default: PermissionDecision,
    ) -> PermissionDecision {
        use sqlx::Row;
        // Two stored rules can legitimately disagree: a prompt's "Always Allow"
        // and a later Settings edit. Ranking `always` first meant the Settings
        // change was silently ignored forever (and there was no way to revoke
        // the always-allow), so the most recently written rule wins, with the
        // old scope order as the tiebreak for rows that predate `updatedAt`.
        let Ok(rows) = sqlx::query(
            "SELECT data FROM permission_rules \
             WHERE json_extract(data, '$.toolId') = ?1 \
             ORDER BY COALESCE(json_extract(data, '$.updatedAt'), '') DESC, \
                      CASE json_extract(data, '$.scope') \
                        WHEN 'always' THEN 0 WHEN 'tool' THEN 1 ELSE 2 END, \
                      id",
        )
        .bind(tool_id)
        .fetch_all(self.storage.pool())
        .await
        else {
            return default;
        };
        for row in rows {
            let raw: String = row.try_get("data").unwrap_or_default();
            let data: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            match data.get("decision").and_then(|v| v.as_str()) {
                Some("allow") => return PermissionDecision::Allow,
                Some("deny") => return PermissionDecision::Deny,
                Some("ask") => return PermissionDecision::Ask,
                _ => continue,
            }
        }
        default
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
            self.remember_rule(tool_id, decision).await?;
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
    pub async fn remember_rule(
        &self,
        tool_id: &str,
        decision: PermissionDecision,
    ) -> anyhow::Result<()> {
        let rule = json!({
            "toolId": tool_id,
            "scope": "always",
            "decision": match decision {
                PermissionDecision::Allow => "allow",
                PermissionDecision::Ask => "ask",
                PermissionDecision::Deny => "deny",
            },
            "pattern": null,
            "updatedAt": Utc::now().to_rfc3339(),
        });
        sqlx::query(
            "INSERT INTO permission_rules (id, owner_id, data) VALUES (?1, ?2, ?3) \
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
        )
        .bind(format!("always:{}", tool_id))
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(rule.to_string())
        .execute(self.storage.pool())
        .await?;
        Ok(())
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
    pub async fn record_approval(
        &self,
        approval_id: &str,
        run_id: Uuid,
        tool_call_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        expires: chrono::DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let data = json!({
            "toolName": tool_name,
            "arguments": arguments,
            "grantedAt": null,
            "remember": false,
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
