//! Command-layer coverage for `budget_extension`.
//!
//! The engine-side rules live in `core/tests/engine_loop.rs`; what is checked
//! here is the boundary the WebView talks to: the run id has to parse, the
//! answer has to reach a Run that is actually waiting, and a rejected answer
//! must surface as an error string instead of being swallowed.

use mix_agent_desktop::commands::runs::{apply_budget_extension, BudgetExtensionInput};
use mixd_core::runs::engine::RunEngine;
use mixd_storage::records::SINGLE_USER_ID;
use mixd_storage::{SecretStore, Storage};
use std::sync::Arc;
use uuid::Uuid;

async fn temp_storage() -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-runs-test-{}", Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

async fn insert_run(storage: &Storage, status: &str) -> Uuid {
    let conversation_id = Uuid::new_v4();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(conversation_id.to_string())
        .bind(SINGLE_USER_ID)
        .bind(serde_json::json!({ "title": "budget" }).to_string())
        .execute(storage.pool())
        .await
        .expect("insert conversation");

    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs (id, owner_id, conversation_id, request_key, data, status) \
         VALUES (?, ?, ?, ?, '{}', ?)",
    )
    .bind(run_id.to_string())
    .bind(SINGLE_USER_ID)
    .bind(conversation_id.to_string())
    .bind(Uuid::new_v4().to_string())
    .bind(status)
    .execute(storage.pool())
    .await
    .expect("insert run");
    run_id
}

fn grant_for(run_id: Uuid) -> BudgetExtensionInput {
    BudgetExtensionInput {
        id: run_id.to_string(),
        grant: true,
        step_limit: Some(42),
        tool_call_limit: Some(7),
        wall_clock_minutes: Some(30),
    }
}

#[tokio::test]
async fn a_budget_answer_for_a_run_that_is_not_pending_is_rejected() {
    let storage = temp_storage().await;
    let engine = Arc::new(RunEngine::with_storage(
        Arc::new(storage),
        Arc::new(SecretStore::new()),
    ));
    let run_id = insert_run(&engine.storage, "running").await;

    let err = apply_budget_extension(&engine, grant_for(run_id))
        .await
        .expect_err("a run that is not waiting must not accept an answer");
    assert!(err.contains("not waiting"), "unexpected error: {err}");

    // The rejection must not have emitted anything either: the WebView shows
    // whatever the run event log contains.
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_events WHERE run_id = ?")
        .bind(run_id.to_string())
        .fetch_one(engine.storage.pool())
        .await
        .expect("count events");
    assert_eq!(stored, 0, "a rejected answer must not be broadcast");
}

#[tokio::test]
async fn a_malformed_run_id_is_rejected_before_it_reaches_the_engine() {
    let storage = temp_storage().await;
    let engine = Arc::new(RunEngine::with_storage(
        Arc::new(storage),
        Arc::new(SecretStore::new()),
    ));

    let err = apply_budget_extension(
        &engine,
        BudgetExtensionInput {
            id: "not-a-uuid".into(),
            grant: true,
            step_limit: None,
            tool_call_limit: None,
            wall_clock_minutes: None,
        },
    )
    .await
    .expect_err("the command must validate the id itself");
    assert!(!err.is_empty());
}

#[tokio::test]
async fn a_pending_run_accepts_the_answer_and_records_it() {
    let storage = temp_storage().await;
    let engine = Arc::new(RunEngine::with_storage(
        Arc::new(storage),
        Arc::new(SecretStore::new()),
    ));
    let run_id = insert_run(&engine.storage, "budget_extension_pending").await;

    apply_budget_extension(&engine, grant_for(run_id))
        .await
        .expect("grant");

    let (status, data): (String, String) =
        sqlx::query_as("SELECT status, data FROM runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_one(engine.storage.pool())
            .await
            .expect("run row");
    assert_eq!(status, "running");
    let data: serde_json::Value = serde_json::from_str(&data).expect("json");
    assert_eq!(data["budgetDecision"], serde_json::json!(true));

    // The answer is broadcast through the same event log the UI replays.
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM run_events WHERE run_id = ? ORDER BY sequence ASC")
            .bind(run_id.to_string())
            .fetch_all(engine.storage.pool())
            .await
            .expect("events");
    assert!(kinds.contains(&"budget_extension_resolved".to_string()));
}
