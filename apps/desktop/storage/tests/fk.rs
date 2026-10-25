//! Foreign keys are declared all over `0001_initial.sql`, and sqlx 0.8 turns
//! `PRAGMA foreign_keys = ON` on for every connection. Whether that is actually
//! true decides whether `delete_conversation` can delete a run that ever made a
//! tool call — so pin it down rather than leaving it to a comment.

use mixd_storage::Storage;
use std::env;

async fn open() -> (Storage, std::path::PathBuf) {
    let dir = env::temp_dir().join(format!("mixd-fk-test-{}", uuid::Uuid::new_v4()));
    let storage = Storage::open(&dir).await.expect("open");
    (storage, dir)
}

#[tokio::test]
async fn foreign_keys_are_enforced() {
    let (storage, dir) = open().await;

    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind("conv-1")
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(r#"{"title":"t"}"#)
        .execute(storage.pool())
        .await
        .expect("insert conversation");

    // A run row with no conversation must be rejected…
    let orphan = sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, request_key) \
         VALUES ('run-1', 'u', '{}', 'missing-conv', 'key-1')",
    )
    .execute(storage.pool())
    .await;
    assert!(
        orphan.is_err(),
        "PRAGMA foreign_keys is ON in sqlx 0.8, so an orphan run must be rejected; \
         got {orphan:?}"
    );

    // …and a tool call pointing at a missing run too.
    let orphan_tool = sqlx::query(
        "INSERT INTO tool_calls (id, owner_id, data, run_id) VALUES ('tc-1', 'u', '{}', 'missing-run')",
    )
    .execute(storage.pool())
    .await;
    assert!(
        orphan_tool.is_err(),
        "tool_calls.run_id must be enforced; got {orphan_tool:?}"
    );

    drop(storage);
    let _ = std::fs::remove_dir_all(&dir);
}
