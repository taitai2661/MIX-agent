use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct ToolView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub builtin: bool,
    /// The decision a call gets when nothing is stored.
    pub default_permission: String,
    /// True when the decision depends on the arguments, as it does for `files`
    /// (read vs write) — so a single "ask" label would be a lie in one direction
    /// and unusable in the other.
    pub mixed: bool,
    pub parameters: serde_json::Value,
}

/// How a tool's default decision behaves: never sensitive, always sensitive, or
/// argument-dependent.
fn sensitivity(tool: &dyn mixd_tools::Tool) -> (String, bool) {
    let read_only = tool.decision_for(&serde_json::json!({ "probe": true }));
    let write = tool.decision_for(&serde_json::json!({
        "probe": true, "action": "write", "old_string": "a", "new_string": "b", "path": "p"
    }));
    let label = |d: mixd_tools::PermissionDecision| match d {
        mixd_tools::PermissionDecision::Allow => "allow".to_string(),
        mixd_tools::PermissionDecision::Ask => "ask".to_string(),
        mixd_tools::PermissionDecision::Deny => "deny".to_string(),
    };
    if read_only == write {
        (label(read_only), false)
    } else {
        (label(read_only), true)
    }
}

#[tauri::command]
pub async fn list_tools(state: State<'_, AppState>) -> Result<Vec<ToolView>, String> {
    // Prefer the live registry; fall back to whatever was persisted.
    let live = state.engine.tools.read().await.list();
    if !live.is_empty() {
        return Ok(live
            .iter()
            .map(|t| {
                let (default_permission, mixed) = sensitivity(t.as_ref());
                ToolView {
                    id: t.id().to_string(),
                    name: t.name().to_string(),
                    description: t.description().to_string(),
                    builtin: true,
                    default_permission,
                    mixed,
                    parameters: t.parameters(),
                }
            })
            .collect());
    }

    let pool = state.storage.pool();
    let rows: Vec<ToolRow> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.name') AS name, \
                json_extract(data, '$.description') AS description, \
                json_extract(data, '$.builtin') AS builtin, \
                json_extract(data, '$.defaultPermission') AS permission, \
                json_extract(data, '$.parameters') AS parameters \
         FROM tools ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, description, builtin, permission, parameters)| ToolView {
                id,
                name,
                description,
                builtin: builtin != 0,
                default_permission: permission,
                // The persisted row predates per-call sensitivity, so it can only
                // report a single decision.
                mixed: false,
                parameters: serde_json::from_str(&parameters).unwrap_or(serde_json::json!({})),
            },
        )
        .collect())
}

type ToolRow = (String, String, String, i64, String, String);

#[derive(Deserialize)]
pub struct PermissionRuleInput {
    pub tool_id: String,
    /// `"global"` | `"workspace"`
    pub scope: String,
    /// `"allow"` | `"ask"` | `"deny"`
    pub decision: String,
    pub pattern: Option<String>,
}

#[tauri::command]
pub async fn set_permission_rule(
    state: State<'_, AppState>,
    input: PermissionRuleInput,
) -> Result<(), String> {
    let rule_id = format!("{}:{}", input.scope, input.tool_id);
    let data = serde_json::json!({
        "toolId": input.tool_id,
        "scope": input.scope,
        "decision": input.decision,
        "pattern": input.pattern,
    });
    sqlx::query(
        "INSERT INTO permission_rules (id, owner_id, data) VALUES (?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
    )
    .bind(&rule_id)
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(data.to_string())
    .execute(state.storage.pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
pub struct PermissionRuleView {
    pub id: String,
    pub tool_id: String,
    pub scope: String,
    pub decision: String,
}

#[tauri::command]
pub async fn list_permission_rules(
    state: State<'_, AppState>,
) -> Result<Vec<PermissionRuleView>, String> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.toolId') AS tool_id, \
                json_extract(data, '$.scope') AS scope, \
                json_extract(data, '$.decision') AS decision \
         FROM permission_rules ORDER BY id",
    )
    .fetch_all(state.storage.pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(id, tool_id, scope, decision)| PermissionRuleView {
            id,
            tool_id,
            scope,
            decision,
        })
        .collect())
}

#[derive(Serialize)]
pub struct PendingApprovalView {
    pub approval_id: String,
    pub run_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub created_at: String,
    pub expires_at: String,
}

#[tauri::command]
pub async fn list_pending_approvals(
    state: State<'_, AppState>,
) -> Result<Vec<PendingApprovalView>, String> {
    let pending = state.engine.executor.list_pending().await;
    Ok(pending
        .into_iter()
        .map(|p| PendingApprovalView {
            approval_id: p.approval_id,
            run_id: p.run_id.to_string(),
            tool_call_id: p.tool_call_id,
            tool_name: p.tool_name,
            arguments: p.arguments,
            created_at: p.created_at.to_rfc3339(),
            expires_at: p.expires_at.to_rfc3339(),
        })
        .collect())
}
