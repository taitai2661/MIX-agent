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

#[derive(Clone)]
pub struct ToolExecutor {
    pub storage: Arc<Storage>,
    pub secrets: Arc<SecretStore>,
    pub pending: Arc<Mutex<Vec<PendingApproval>>>,
    /// Wakes the paused Run when the user answers a prompt.
    waiters: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalOutcome>>>>,
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
        let Ok(rows) = sqlx::query(
            "SELECT data FROM permission_rules \
             WHERE json_extract(data, '$.toolId') = ?1 \
            ORDER BY CASE json_extract(data, '$.scope') \
                       WHEN 'always' THEN 0 WHEN 'tool' THEN 1 ELSE 2 END",
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
        if let Some(tx) = self.waiters.lock().await.remove(approval_id) {
            let _ = tx.send(outcome);
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

    /// The prompts the UI should render right now, dropping expired ones.
    pub async fn list_pending(&self) -> Vec<PendingApproval> {
        let now = Utc::now();
        let mut guard = self.pending.lock().await;
        guard.retain(|p| p.expires_at > now);
        guard.clone()
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
        self.waiters
            .lock()
            .await
            .insert(approval.approval_id.clone(), tx);
        self.pending.lock().await.push(approval.clone());
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
