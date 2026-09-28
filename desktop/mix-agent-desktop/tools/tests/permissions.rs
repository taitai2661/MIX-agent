//! Permission gate: stored rules decide whether a Tool runs, prompts block the
//! Run until answered, and "Always Allow" survives into later calls.

use mixd_storage::{SecretStore, Storage};
use mixd_tools::approval::ApprovalOutcome;
use mixd_tools::execute::ToolExecutor;
use mixd_tools::permission::PermissionDecision;
use std::env;
use std::sync::Arc;
use uuid::Uuid;

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
            .resolve_permission(TOOL, PermissionDecision::Ask)
            .await,
        PermissionDecision::Ask
    );
    assert_eq!(
        executor
            .resolve_permission(TOOL, PermissionDecision::Allow)
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
            .resolve_permission(TOOL, PermissionDecision::Allow)
            .await,
        PermissionDecision::Deny,
        "a deny rule must win over the tool default"
    );
    assert_ne!(
        executor
            .resolve_permission("web_fetch", PermissionDecision::Ask)
            .await,
        PermissionDecision::Deny,
        "rules must not leak across tools"
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
            .resolve_permission(TOOL, PermissionDecision::Ask)
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
            .resolve_permission(TOOL, PermissionDecision::Allow)
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
