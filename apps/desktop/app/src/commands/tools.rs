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
    .map_err(crate::error::cmd_err)?;
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
    #[serde(default)]
    pub pattern: Option<String>,
    /// Required when `scope == "workspace"`; ignored otherwise.
    #[serde(default)]
    pub workspace_id: Option<String>,
}

#[tauri::command]
pub async fn set_permission_rule(
    state: State<'_, AppState>,
    input: PermissionRuleInput,
) -> Result<(), String> {
    let scope = parse_scope(&input.scope)?;
    let decision = parse_decision(&input.decision)?;
    let pattern = input
        .pattern
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string());
    // Known-tool gate: a typo'd tool_id would otherwise create a rule that
    // never matches, silently leaving the caller unprotected.
    let known = state.engine.tools.read().await.list();
    if !known.iter().any(|t| t.id() == input.tool_id) {
        return Err(format!("unknown tool: {}", input.tool_id));
    }
    // Glob gate: reject patterns that can never match so a permissive
    // fallback does not silently take over.
    if let Some(p) = pattern.as_deref() {
        if p.len() > 1024 {
            return Err("pattern is too long".to_string());
        }
        if !mixd_tools::is_valid_pattern(p) {
            return Err(format!("invalid glob pattern: {p}"));
        }
    }
    // Confirmation gate: a patternless global allow opens every tool call.
    if scope == mixd_tools::RuleScope::Global
        && decision == mixd_tools::PermissionDecision::Allow
        && pattern.is_none()
    {
        return Err(
            "global allow without a pattern is too broad; scope it to a workspace or add a pattern"
                .to_string(),
        );
    }
    if scope == mixd_tools::RuleScope::Workspace && input.workspace_id.is_none() {
        return Err("workspace_id is required for workspace-scoped rules".into());
    }
    let workspace_id = match scope {
        mixd_tools::RuleScope::Workspace => input.workspace_id.unwrap_or_default(),
        // `parse_scope` never produces Session (it is in-memory only), but the
        // enum is shared with the executor.
        mixd_tools::RuleScope::Global | mixd_tools::RuleScope::Session => String::new(),
    };
    // Rule-id hardening: the old `scope:tool:workspace` form both collided
    // across patterns and allowed `:` in the tool/pattern to forge a
    // different row's id. Escape separators and mix the pattern into a hash
    // so each (scope, tool, workspace, pattern) maps to one row.
    let rule_id = permission_rule_id(
        scope.as_str(),
        &input.tool_id,
        &workspace_id,
        pattern.as_deref(),
    );
    let data = serde_json::json!({
        "toolId": input.tool_id,
        "scope": scope.as_str(),
        "decision": match decision {
            mixd_tools::PermissionDecision::Allow => "allow",
            mixd_tools::PermissionDecision::Ask => "ask",
            mixd_tools::PermissionDecision::Deny => "deny",
        },
        "pattern": pattern,
        "workspaceId": if scope == mixd_tools::RuleScope::Workspace {
            serde_json::Value::String(workspace_id.clone())
        } else {
            serde_json::Value::Null
        },
        "updatedAt": chrono::Utc::now().to_rfc3339(),
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
    .map_err(crate::error::cmd_err)?;
    Ok(())
}

fn parse_scope(value: &str) -> Result<mixd_tools::RuleScope, String> {
    match value {
        "global" | "always" => Ok(mixd_tools::RuleScope::Global),
        "workspace" => Ok(mixd_tools::RuleScope::Workspace),
        other => Err(format!("unknown permission scope: {other}")),
    }
}

fn parse_decision(value: &str) -> Result<mixd_tools::PermissionDecision, String> {
    match value {
        "allow" => Ok(mixd_tools::PermissionDecision::Allow),
        "ask" => Ok(mixd_tools::PermissionDecision::Ask),
        "deny" => Ok(mixd_tools::PermissionDecision::Deny),
        other => Err(format!("unknown permission decision: {other}")),
    }
}

fn sanitize_rule_part(s: &str) -> String {
    s.replace('%', "%25").replace(':', "%3A")
}

/// Stable FNV-1a hash for persisted rule ids.
///
/// `DefaultHasher` is explicitly unspecified across Rust releases, so using it
/// for a primary key would mint a new row on every toolchain upgrade and leave
/// the old rule behind. This hash only needs to be deterministic, not
/// cryptographic.
fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn permission_rule_id(
    scope: &str,
    tool_id: &str,
    workspace_id: &str,
    pattern: Option<&str>,
) -> String {
    // The pattern is hashed, so `:` inside it cannot forge another row's id.
    // Length-prefixing the other escaped parts keeps `a:b` vs `a` + `b` apart.
    let raw = [
        sanitize_rule_part(scope),
        sanitize_rule_part(tool_id),
        sanitize_rule_part(workspace_id),
        pattern.unwrap_or("").to_string(),
    ]
    .map(|part| format!("{}:{part}", part.len()))
    .join("|");
    format!(
        "{}:{}:{}:{:016x}",
        sanitize_rule_part(scope),
        sanitize_rule_part(tool_id),
        sanitize_rule_part(workspace_id),
        stable_hash(&raw)
    )
}

#[derive(Serialize)]
pub struct PermissionRuleView {
    pub id: String,
    pub tool_id: String,
    pub scope: String,
    pub decision: String,
    pub pattern: Option<String>,
    pub workspace_id: Option<String>,
}

#[tauri::command]
pub async fn list_permission_rules(
    state: State<'_, AppState>,
) -> Result<Vec<PermissionRuleView>, String> {
    let rows: Vec<PermissionRuleRow> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.toolId') AS tool_id, \
                json_extract(data, '$.scope') AS scope, \
                json_extract(data, '$.decision') AS decision, \
                json_extract(data, '$.pattern') AS pattern, \
                json_extract(data, '$.workspaceId') AS workspace_id \
         FROM permission_rules ORDER BY id",
    )
    .fetch_all(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, tool_id, scope, decision, pattern, workspace_id)| PermissionRuleView {
                id,
                tool_id,
                scope,
                decision,
                pattern,
                workspace_id,
            },
        )
        .collect())
}

type PermissionRuleRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

#[derive(Serialize)]
pub struct PendingApprovalView {
    pub approval_id: String,
    pub run_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub created_at: String,
    pub expires_at: String,
    /// The task this prompt belongs to. A security decision that does not name
    /// its conversation/folder invites rubber-stamping: "常に許可" then applies
    /// to a task the user cannot identify.
    pub conversation_id: Option<String>,
    pub conversation_title: Option<String>,
    /// Mode of the thread so the UI can switch to the right surface without
    /// looking the conversation up in a bounded list.
    pub conversation_mode: Option<String>,
}

#[tauri::command]
pub async fn list_pending_approvals(
    state: State<'_, AppState>,
) -> Result<Vec<PendingApprovalView>, String> {
    let pending = state.engine.executor.list_pending().await;
    let mut out = Vec::with_capacity(pending.len());
    for p in pending {
        let run_id = p.run_id.to_string();
        // One point lookup per pending prompt (there are only ever a handful).
        // The join is what gives the approval queue its task context.
        let context: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT r.conversation_id, json_extract(c.data, '$.title'), \
                    json_extract(c.data, '$.mode') \
             FROM runs r LEFT JOIN conversations c ON c.id = r.conversation_id \
             WHERE r.id = ?",
        )
        .bind(&run_id)
        .fetch_optional(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
        let (conversation_id, conversation_title, conversation_mode) =
            context.unwrap_or((None, None, None));
        out.push(PendingApprovalView {
            approval_id: p.approval_id,
            run_id,
            tool_call_id: p.tool_call_id,
            tool_name: p.tool_name,
            arguments: p.arguments,
            created_at: p.created_at.to_rfc3339(),
            expires_at: p.expires_at.to_rfc3339(),
            conversation_id,
            conversation_title,
            conversation_mode,
        });
    }
    Ok(out)
}
