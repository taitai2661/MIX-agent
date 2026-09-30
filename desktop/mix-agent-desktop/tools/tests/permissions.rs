//! Permission gate: stored rules decide whether a Tool runs, prompts block the
//! Run until answered, and "Always Allow" survives into later calls.

use mixd_storage::{SecretStore, Storage};
use mixd_tools::approval::ApprovalOutcome;
use mixd_tools::execute::{PermissionContext, ToolExecutor};
use mixd_tools::permission::PermissionDecision;
use std::env;
use std::sync::Arc;
use uuid::Uuid;

/// A bare context, with no workspace and no pattern target — the call shape
/// used by every legacy test. Newer tests construct their own context.
fn ctx() -> PermissionContext<'static> {
    PermissionContext {
        workspace_id: None,
        pattern_target: None,
    }
}

async fn executor() -> (ToolExecutor, std::path::PathBuf) {
    let dir = env::temp_dir().join(format!("mixd-perm-test-{}", Uuid::new_v4()));
    let storage = Arc::new(Storage::open(&dir).await.expect("open"));
    // The permission gate never touches the Keychain, so an in-memory store is
    // enough and keeps the tests off the user's real keychain.
    let secrets = Arc::new(SecretStore::new());
    (ToolExecutor::new(storage, secrets), dir)
}

const TOOL: &str = "terminal";

/// `approvals` references both `runs` and `tool_calls`, so a prompt can only be
/// opened for a call that is already on the timeline — same as in the engine.
async fn seed_call(executor: &ToolExecutor, tool_call_id: &str) -> Uuid {
    let pool = executor.storage.pool();
    let conversation_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?1, ?2, '{}')")
        .bind(&conversation_id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .execute(pool)
        .await
        .expect("seed conversation");

    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?1, ?2, '{}', ?3, 'running', ?4)",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(&conversation_id)
    .bind(tool_call_id)
    .execute(pool)
    .await
    .expect("seed run");
    executor
        .record_tool_call(
            run_id,
            tool_call_id,
            TOOL,
            serde_json::json!({}),
            "awaiting_approval",
            None,
            None,
        )
        .await
        .expect("seed tool call");
    run_id
}

#[tokio::test]
async fn falls_back_to_the_tool_default_when_no_rule_exists() {
    let (executor, _dir) = executor().await;
    assert_eq!(
        executor
            .resolve_permission(TOOL, &ctx(), PermissionDecision::Ask)
            .await,
        PermissionDecision::Ask
    );
    assert_eq!(
        executor
            .resolve_permission(TOOL, &ctx(), PermissionDecision::Allow)
            .await,
        PermissionDecision::Allow
    );
}

#[tokio::test]
async fn stored_rule_overrides_the_default() {
    let (executor, _dir) = executor().await;
    executor
        .remember_rule(TOOL, PermissionDecision::Deny)
        .await
        .expect("write rule");
    assert_eq!(
        executor
            .resolve_permission(TOOL, &ctx(), PermissionDecision::Allow)
            .await,
        PermissionDecision::Deny,
        "a deny rule must win over the tool default"
    );
    assert_ne!(
        executor
            .resolve_permission("web_fetch", &ctx(), PermissionDecision::Ask)
            .await,
        PermissionDecision::Deny,
        "rules must not leak across tools"
    );
}

#[tokio::test]
async fn a_later_settings_edit_beats_an_older_always_allow() {
    let (executor, _dir) = executor().await;
    executor
        .remember_rule(TOOL, PermissionDecision::Allow)
        .await
        .expect("always allow");
    // The user then changes the same tool to `deny` in Settings, which writes a
    // `global` scoped row. Ranking `always` first made this a no-op, with no
    // way left to revoke the always-allow.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    sqlx::query(
        "INSERT INTO permission_rules (id, owner_id, data) VALUES (?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
    )
    .bind(format!("global:{TOOL}"))
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(
        serde_json::json!({
            "toolId": TOOL,
            "scope": "global",
            "decision": "deny",
            "updatedAt": chrono::Utc::now().to_rfc3339(),
        })
        .to_string(),
    )
    .execute(executor.storage.pool())
    .await
    .expect("settings rule");
    assert_eq!(
        executor
            .resolve_permission(TOOL, &ctx(), PermissionDecision::Ask)
            .await,
        PermissionDecision::Deny,
        "the newer Settings rule must win over the older always-allow"
    );
}

#[tokio::test]
async fn pending_prompt_blocks_until_answered() {
    let (executor, _dir) = executor().await;
    let run_id = seed_call(&executor, "call-1").await;
    let (rx, _expires) = executor
        .request_approval(run_id, "call-1", TOOL, serde_json::json!({"cmd": "ls"}))
        .await
        .expect("prompt");

    let pending = executor.list_pending().await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tool_name, TOOL);
    let approval_id = pending[0].approval_id.clone();

    // Resolving with "Always Allow" must both wake the Run and persist a rule
    // keyed on the Tool — not on the approval id.
    executor
        .resolve_approval(
            &approval_id,
            TOOL,
            ApprovalOutcome::Approve { remember: true },
        )
        .await
        .expect("resolve");

    assert_eq!(
        rx.await.expect("outcome delivered"),
        ApprovalOutcome::Approve { remember: true }
    );
    assert!(executor.list_pending().await.is_empty(), "prompt cleared");
    assert_eq!(
        executor
            .resolve_permission(TOOL, &ctx(), PermissionDecision::Ask)
            .await,
        PermissionDecision::Allow,
        "Always Allow must apply to later calls without prompting"
    );
}

#[tokio::test]
async fn deny_wakes_the_run_without_creating_a_rule() {
    let (executor, _dir) = executor().await;
    let run_id = seed_call(&executor, "call-2").await;
    let (rx, _expires) = executor
        .request_approval(run_id, "call-2", TOOL, serde_json::json!({}))
        .await
        .expect("prompt");
    let approval_id = executor.list_pending().await[0].approval_id.clone();

    executor
        .resolve_approval(&approval_id, TOOL, ApprovalOutcome::Deny)
        .await
        .expect("resolve");

    assert_eq!(rx.await.expect("outcome delivered"), ApprovalOutcome::Deny);
    assert_eq!(
        executor
            .resolve_permission(TOOL, &ctx(), PermissionDecision::Allow)
            .await,
        PermissionDecision::Allow,
        "a one-off denial must not change the standing rule"
    );
}

#[tokio::test]
async fn expired_prompts_are_not_listed() {
    let (executor, _dir) = executor().await;
    let run_id = seed_call(&executor, "call-3").await;
    executor
        .request_approval(run_id, "call-3", TOOL, serde_json::json!({}))
        .await
        .expect("prompt");
    // Force the prompt to look stale.
    executor.pending.lock().await[0].expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
    assert!(executor.list_pending().await.is_empty());
}

/// An expired prompt used to survive in two places at once: the engine's
/// `select!` timeout branch only stops *waiting* (it never calls
/// `resolve_approval`), so the `waiters` channel stayed in the map and the
/// `approvals` row stayed `pending` forever.
#[tokio::test]
async fn an_expired_prompt_is_reclaimed_from_the_map_and_the_database() {
    let (executor, _dir) = executor().await;
    let run_id = seed_call(&executor, "call-expired").await;
    let (_rx, _expires) = executor
        .request_approval(
            run_id,
            "call-expired",
            TOOL,
            serde_json::json!({"cmd": "rm -rf"}),
        )
        .await
        .expect("prompt");
    let approval_id = executor.list_pending().await[0].approval_id.clone();
    assert_eq!(executor.open_waiters().await, 1);

    // Simulate the TTL elapsing without anyone answering.
    executor.pending.lock().await[0].expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);

    assert!(
        executor.list_pending().await.is_empty(),
        "an expired prompt must not be offered to the UI"
    );
    assert_eq!(
        executor.open_waiters().await,
        0,
        "the wake-up channel must be dropped with the prompt"
    );

    let status: String = sqlx::query_scalar("SELECT status FROM approvals WHERE id = ?")
        .bind(&approval_id)
        .fetch_one(executor.storage.pool())
        .await
        .expect("approval row");
    assert_eq!(status, "expired", "the row must not stay pending");

    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM approvals WHERE status = 'pending'")
            .fetch_one(executor.storage.pool())
            .await
            .expect("count");
    assert_eq!(remaining, 0, "no pending approvals may outlive their TTL");

    // Sweeping twice must be harmless: the row is already closed and the
    // prompt is gone from memory.
    executor.sweep_expired().await;
    assert!(executor.list_pending().await.is_empty());
    assert_eq!(executor.open_waiters().await, 0);
}

/// Answering normally must clear all three copies too, so the sweep cannot be
/// the only path that keeps the structures in step.
#[tokio::test]
async fn resolving_a_prompt_clears_the_waiter_and_the_row() {
    let (executor, _dir) = executor().await;
    let run_id = seed_call(&executor, "call-resolved").await;
    let (rx, _expires) = executor
        .request_approval(run_id, "call-resolved", TOOL, serde_json::json!({}))
        .await
        .expect("prompt");
    let approval_id = executor.list_pending().await[0].approval_id.clone();
    assert_eq!(executor.open_waiters().await, 1);

    executor
        .resolve_approval(&approval_id, TOOL, ApprovalOutcome::Deny)
        .await
        .expect("resolve");
    rx.await.expect("outcome delivered");

    assert!(executor.list_pending().await.is_empty());
    assert_eq!(executor.open_waiters().await, 0);
    let status: String = sqlx::query_scalar("SELECT status FROM approvals WHERE id = ?")
        .bind(&approval_id)
        .fetch_one(executor.storage.pool())
        .await
        .expect("approval row");
    assert_eq!(status, "denied");
}

#[tokio::test]
async fn tool_name_is_recoverable_after_the_prompt_is_gone() {
    let (executor, _dir) = executor().await;
    let run_id = seed_call(&executor, "call-4").await;
    let (rx, _expires) = executor
        .request_approval(run_id, "call-4", "browser_action", serde_json::json!({}))
        .await
        .expect("prompt");
    let approval_id = executor.list_pending().await[0].approval_id.clone();
    drop(rx);
    executor.pending.lock().await.clear();

    assert_eq!(
        executor
            .tool_name_for_approval(&approval_id)
            .await
            .as_deref(),
        Some("browser_action"),
        "must fall back to the persisted approvals row"
    );
}

/// Helpers that insert a single rule row directly, bypassing `remember_rule`,
/// so we can pin every field (scope, pattern, decision, workspace id) the
/// tests want to verify.
async fn write_rule(
    executor: &ToolExecutor,
    id: &str,
    tool_id: &str,
    scope: &str,
    decision: &str,
    pattern: Option<&str>,
    workspace_id: Option<&str>,
) {
    let mut data = serde_json::json!({
        "toolId": tool_id,
        "scope": scope,
        "decision": decision,
        "pattern": pattern,
        "updatedAt": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(ws) = workspace_id {
        data["workspaceId"] = serde_json::Value::String(ws.to_string());
    }
    sqlx::query(
        "INSERT INTO permission_rules (id, owner_id, data) VALUES (?, ?, ?) \
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
    )
    .bind(id)
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(data.to_string())
    .execute(executor.storage.pool())
    .await
    .expect("write rule");
}

#[tokio::test]
async fn a_pattern_rule_only_matches_when_the_tool_target_fits() {
    let (executor, _dir) = executor().await;
    write_rule(
        &executor,
        "global:files:secret",
        "files",
        "global",
        "allow",
        Some("**/*.secret"),
        None,
    )
    .await;

    let allow_ctx = PermissionContext {
        workspace_id: None,
        pattern_target: Some("keys/api.secret"),
    };
    assert_eq!(
        executor
            .resolve_permission("files", &allow_ctx, PermissionDecision::Ask)
            .await,
        PermissionDecision::Allow,
        "a glob pattern must match its target"
    );

    let miss_ctx = PermissionContext {
        workspace_id: None,
        pattern_target: Some("src/main.rs"),
    };
    assert_eq!(
        executor
            .resolve_permission("files", &miss_ctx, PermissionDecision::Ask)
            .await,
        PermissionDecision::Ask,
        "a pattern rule must not widen access to other paths"
    );

    let no_target_ctx = PermissionContext {
        workspace_id: None,
        pattern_target: None,
    };
    assert_eq!(
        executor
            .resolve_permission("files", &no_target_ctx, PermissionDecision::Ask)
            .await,
        PermissionDecision::Ask,
        "a pattern rule with no target exposed by the tool falls back to default"
    );
}

#[tokio::test]
async fn a_workspace_deny_beats_a_global_allow() {
    let (executor, _dir) = executor().await;
    let workspace = Uuid::new_v4();

    write_rule(
        &executor,
        "global:files",
        "files",
        "global",
        "allow",
        None,
        None,
    )
    .await;
    write_rule(
        &executor,
        &format!("workspace:files:{workspace}"),
        "files",
        "workspace",
        "deny",
        None,
        Some(&workspace.to_string()),
    )
    .await;

    let ctx = PermissionContext {
        workspace_id: Some(workspace),
        pattern_target: None,
    };
    assert_eq!(
        executor
            .resolve_permission("files", &ctx, PermissionDecision::Ask)
            .await,
        PermissionDecision::Deny,
        "the workspace deny must win over the global allow"
    );

    let other_workspace_ctx = PermissionContext {
        workspace_id: Some(Uuid::new_v4()),
        pattern_target: None,
    };
    assert_eq!(
        executor
            .resolve_permission("files", &other_workspace_ctx, PermissionDecision::Ask)
            .await,
        PermissionDecision::Allow,
        "an unrelated workspace still falls back to the global allow"
    );
}

#[tokio::test]
async fn workspace_rules_do_not_apply_to_calls_without_a_workspace() {
    let (executor, _dir) = executor().await;
    let workspace = Uuid::new_v4();
    write_rule(
        &executor,
        &format!("workspace:files:{workspace}"),
        "files",
        "workspace",
        "deny",
        None,
        Some(&workspace.to_string()),
    )
    .await;
    // A chat that isn't bound to any workspace must not be denied by a
    // workspace-scoped row, even if the workspace happens to exist.
    assert_eq!(
        executor
            .resolve_permission(
                "files",
                &PermissionContext {
                    workspace_id: None,
                    pattern_target: None,
                },
                PermissionDecision::Allow,
            )
            .await,
        PermissionDecision::Allow
    );
}
