//! Integration coverage for the history read path (`list_messages`,
//! `list_conversations` preview, `list_run_events`) against a temporary
//! SQLite database — the same shape `dispatch_run` writes.

use mix_agent_desktop::commands::conversations::{
    fetch_conversation_summaries, fetch_messages, purge_temporary_run, truncate_messages_from,
};
use mix_agent_desktop::commands::runs::fetch_run_events;
use mixd_storage::records::SINGLE_USER_ID;
use mixd_storage::Storage;
use sqlx::Row;

async fn temp_storage() -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-app-test-{}", uuid::Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

async fn insert_conversation(pool: &sqlx::Pool<sqlx::Sqlite>, id: &str, title: &str) {
    let data = serde_json::json!({
        "title": title,
        "mode": "agent",
        "modelId": "prov:model-x",
        "updatedAt": "2026-01-02T03:04:05Z",
        "state": "active",
    });
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(id)
        .bind(SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(pool)
        .await
        .expect("insert conversation");
}

async fn insert_message(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    id: &str,
    conversation_id: &str,
    data: serde_json::Value,
) {
    sqlx::query("INSERT INTO messages (id, owner_id, data, conversation_id) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind(SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(conversation_id)
        .execute(pool)
        .await
        .expect("insert message");
}

#[tokio::test]
async fn list_messages_returns_history_with_reasoning_and_attachments() {
    let storage = temp_storage().await;
    let pool = storage.pool();
    insert_conversation(pool, "conv-1", "History test").await;

    insert_message(
        pool,
        "m-1",
        "conv-1",
        serde_json::json!({
            "role": "user",
            "content": "こんにちは",
            "modelId": "prov:model-x",
            "runId": "run-1",
            "kind": "user",
            "attachments": [{
                "kind": "image",
                "mimeType": "image/png",
                "dataBase64": "AAAA",
                "filename": "chart.png"
            }],
        }),
    )
    .await;
    insert_message(
        pool,
        "m-2",
        "conv-1",
        serde_json::json!({
            "role": "assistant",
            "content": "了解しました",
            "reasoning": "まず日本語を確認",
            "modelId": "prov:model-x",
            "runId": "run-1",
            "kind": "assistant",
        }),
    )
    .await;

    let messages = fetch_messages(pool, "conv-1", 100).await.expect("fetch");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "こんにちは");
    assert_eq!(messages[0].attachments.len(), 1);
    assert_eq!(messages[0].attachments[0].mime_type, "image/png");
    assert_eq!(
        messages[0].attachments[0].filename.as_deref(),
        Some("chart.png")
    );
    assert_eq!(messages[1].role, "assistant");
    // `reasoning` is only in the raw JSON payload, not in `MessageData`.
    assert_eq!(messages[1].reasoning.as_deref(), Some("まず日本語を確認"));
    assert_eq!(messages[1].run_id.as_deref(), Some("run-1"));

    // Ordering is created_at ASC; same-millisecond ties fall back to rowid.
    assert!(messages[0].id == "m-1");
}

#[tokio::test]
async fn list_messages_limit_is_applied() {
    let storage = temp_storage().await;
    let pool = storage.pool();
    insert_conversation(pool, "conv-2", "Limit test").await;
    for i in 0..5 {
        insert_message(
            pool,
            &format!("m-{i}"),
            "conv-2",
            serde_json::json!({ "role": "user", "content": i.to_string(), "kind": "user" }),
        )
        .await;
        // Force distinct created_at values so ordering is deterministic.
        sqlx::query("UPDATE messages SET created_at = ? WHERE id = ?")
            .bind(format!("2026-01-01T00:00:0{i}.000Z"))
            .bind(format!("m-{i}"))
            .execute(pool)
            .await
            .expect("stamp");
    }

    let limited = fetch_messages(pool, "conv-2", 3).await.expect("fetch");
    assert_eq!(limited.len(), 3);
    // The *newest* three, returned oldest-first. Keeping the oldest three meant
    // a long conversation showed the user only its first lines forever.
    assert_eq!(limited[0].content, "2");
    assert_eq!(limited[1].content, "3");
    assert_eq!(limited[2].content, "4");
}

#[tokio::test]
async fn conversation_summaries_carry_preview_and_count() {
    let storage = temp_storage().await;
    let pool = storage.pool();
    insert_conversation(pool, "conv-3", "Preview test").await;
    insert_message(
        pool,
        "m-10",
        "conv-3",
        serde_json::json!({ "role": "user", "content": "最初", "kind": "user" }),
    )
    .await;
    insert_message(
        pool,
        "m-11",
        "conv-3",
        serde_json::json!({ "role": "assistant", "content": "最後の返答", "kind": "assistant" }),
    )
    .await;

    let summaries = fetch_conversation_summaries(pool).await.expect("fetch");
    let conv = summaries.iter().find(|s| s.id == "conv-3").expect("found");
    assert_eq!(conv.title, "Preview test");
    assert_eq!(conv.message_count, 2);
    // Preview comes from the newest message.
    assert_eq!(conv.preview.as_deref(), Some("最後の返答"));
}

#[tokio::test]
async fn run_events_are_returned_in_sequence_order() {
    let storage = temp_storage().await;
    let pool = storage.pool();
    insert_conversation(pool, "conv-x", "Run events test").await;
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES ('run-9', ?, '{}', 'conv-x', 'completed', 'rk')",
    )
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert run");

    // Insert out of order; the query must sort by sequence.
    for (id, seq, kind) in [
        ("e-3", 3, "text"),
        ("e-1", 1, "status"),
        ("e-2", 2, "tool_started"),
    ] {
        sqlx::query(
            "INSERT INTO run_events (id, run_id, sequence, kind, data) VALUES (?, 'run-9', ?, ?, ?)",
        )
        .bind(id)
        .bind(seq)
        .bind(kind)
        .bind(serde_json::json!({ "delta": "x" }).to_string())
        .execute(pool)
        .await
        .expect("insert event");
    }

    let events = fetch_run_events(pool, "run-9").await.expect("fetch");
    assert_eq!(events.len(), 3);
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, vec!["status", "tool_started", "text"]);
    assert!(events[0].data.is_object());
    assert_eq!(events[0].run_id, "run-9");

    // Unknown run yields an empty list, not an error.
    let empty = fetch_run_events(pool, "run-missing").await.expect("fetch");
    assert!(empty.is_empty());
}

#[tokio::test]
async fn truncate_messages_from_drops_the_turn_and_its_run() {
    let storage = temp_storage().await;
    let pool = storage.pool();
    insert_conversation(pool, "conv-edit", "Edit test").await;

    // Three turns, each with a distinct timestamp and its own Run.
    for i in 0..3 {
        let run = format!("run-{i}");
        sqlx::query(
            "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
             VALUES (?, ?, '{}', 'conv-edit', 'completed', ?)",
        )
        .bind(&run)
        .bind(SINGLE_USER_ID)
        .bind(format!("rk-{i}"))
        .execute(pool)
        .await
        .expect("insert run");
        insert_message(
            pool,
            &format!("m-{i}"),
            "conv-edit",
            serde_json::json!({
                "role": "user",
                "content": format!("turn {i}"),
                "runId": run,
                "kind": "user",
            }),
        )
        .await;
        sqlx::query("UPDATE messages SET created_at = ? WHERE id = ?")
            .bind(format!("2026-01-01T00:00:0{i}.000Z"))
            .bind(format!("m-{i}"))
            .execute(pool)
            .await
            .expect("stamp");
        sqlx::query(
            "INSERT INTO run_events (id, run_id, sequence, kind, data) VALUES (?, ?, 1, 'text', '{}')",
        )
        .bind(format!("e-{i}"))
        .bind(&run)
        .execute(pool)
        .await
        .expect("insert event");
    }

    // Branch from the second turn: it and everything after it go.
    let rowid: i64 = sqlx::query_scalar("SELECT rowid FROM messages WHERE id = 'm-1'")
        .fetch_one(pool)
        .await
        .expect("target rowid");
    let mut conn = pool.acquire().await.expect("acquire");
    truncate_messages_from(&mut conn, "conv-edit", "2026-01-01T00:00:01.000Z", rowid)
        .await
        .expect("truncate");
    drop(conn);

    let messages = fetch_messages(pool, "conv-edit", 100).await.expect("fetch");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "turn 0");

    // The edited turn's Run and its events go with the transcript, so the
    // activity panel cannot show a Run that has nothing behind it.
    let runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE conversation_id = 'conv-edit'")
            .fetch_one(pool)
            .await
            .expect("count runs");
    assert_eq!(runs, 1);
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM run_events")
        .fetch_one(pool)
        .await
        .expect("count events");
    assert_eq!(events, 1);
}

/// `PRAGMA foreign_keys` is ON (see `storage/tests/fk.rs`), so the truncation
/// has to remove `approvals` before `tool_calls` and both before `runs`. With
/// the old leaf-last ordering, branching away from *any* turn that had ever
/// been approved failed with `FOREIGN KEY constraint failed` and rolled the
/// whole edit back.
#[tokio::test]
async fn truncate_drops_tool_calls_approvals_and_feedback_before_their_parents() {
    let storage = temp_storage().await;
    let pool = storage.pool();
    insert_conversation(pool, "conv-fk", "FK test").await;

    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES ('run-fk', ?, '{}', 'conv-fk', 'completed', 'rk-fk')",
    )
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert run");

    sqlx::query(
        "INSERT INTO tool_calls (id, owner_id, data, run_id, status) \
         VALUES ('tc-fk', ?, '{}', 'run-fk', 'done')",
    )
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert tool call");

    sqlx::query(
        "INSERT INTO approvals (id, owner_id, data, run_id, tool_call_id, status, expires) \
         VALUES ('ap-fk', ?, '{}', 'run-fk', 'tc-fk', 'approved', '2099-01-01T00:00:00Z')",
    )
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert approval");

    insert_message(
        pool,
        "m-fk",
        "conv-fk",
        serde_json::json!({ "role": "assistant", "content": "hi", "runId": "run-fk" }),
    )
    .await;
    sqlx::query("UPDATE messages SET created_at = ? WHERE id = ?")
        .bind("2026-01-01T00:00:05.000Z")
        .bind("m-fk")
        .execute(pool)
        .await
        .expect("stamp");

    sqlx::query(
        "INSERT INTO message_feedback (id, owner_id, data, message_id) VALUES (?, ?, '{}', 'm-fk')",
    )
    .bind("fb-fk")
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert feedback");

    let rowid: i64 = sqlx::query_scalar("SELECT rowid FROM messages WHERE id = 'm-fk'")
        .fetch_one(pool)
        .await
        .expect("target rowid");
    let mut conn = pool.acquire().await.expect("acquire");
    truncate_messages_from(&mut conn, "conv-fk", "2026-01-01T00:00:05.000Z", rowid)
        .await
        .expect("truncate must succeed despite tool_calls/approvals/feedback");
    drop(conn);

    for table in ["runs", "tool_calls", "approvals", "message_feedback"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .expect("count");
        assert_eq!(count, 0, "{table} must be emptied by the cascade");
    }
    let messages: i64 = sqlx::query_scalar("SELECT count(*) FROM messages")
        .fetch_one(pool)
        .await
        .expect("count messages");
    assert_eq!(messages, 0);
}

/// `delete_run_cascade` is the shared leaf-first delete used by truncation and
/// by conversation deletion; it must unlink the nullable history references
/// rather than fail on them.
#[tokio::test]
async fn delete_run_cascade_unlinks_nullable_history_and_removes_the_run() {
    let storage = temp_storage().await;
    let pool = storage.pool();
    insert_conversation(pool, "conv-cascade", "Cascade").await;

    sqlx::query("INSERT INTO scheduled_jobs (id, owner_id, data) VALUES ('job-1', ?, '{}')")
        .bind(SINGLE_USER_ID)
        .execute(pool)
        .await
        .expect("insert job");
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES ('run-c', ?, '{}', 'conv-cascade', 'completed', 'rk-c')",
    )
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert run");
    sqlx::query(
        "INSERT INTO scheduled_runs (id, owner_id, data, job_id, scheduled_at, status, run_id) \
         VALUES ('sr-1', ?, '{}', 'job-1', '2026-01-01T00:00:00Z', 'done', 'run-c')",
    )
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert scheduled run");
    sqlx::query(
        "INSERT INTO memory_processing_jobs (id, owner_id, data, run_id, status) \
         VALUES ('mpj-1', ?, '{}', 'run-c', 'pending')",
    )
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert memory job");

    let mut conn = pool.acquire().await.expect("acquire");
    mix_agent_desktop::commands::conversations::delete_run_cascade(&mut conn, "run-c")
        .await
        .expect("cascade delete");

    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&mut *conn)
        .await
        .expect("count runs");
    assert_eq!(runs, 0);
    // The scheduled job's own history row survives, just unlinked.
    let linked: Option<String> =
        sqlx::query_scalar("SELECT run_id FROM scheduled_runs WHERE id = 'sr-1'")
            .fetch_one(&mut *conn)
            .await
            .expect("read link");
    assert_eq!(
        linked, None,
        "scheduled_runs.run_id must be unlinked, not deleted"
    );
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM memory_processing_jobs")
        .fetch_one(&mut *conn)
        .await
        .expect("count memory jobs");
    assert_eq!(
        jobs, 0,
        "memory_processing_jobs.run_id is NOT NULL and goes"
    );
}

#[tokio::test]
async fn messages_index_exists_for_conversation_created_at() {
    let storage = temp_storage().await;
    let row = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'messages_conv_created_idx'",
    )
    .fetch_one(storage.pool())
    .await
    .expect("index row");
    assert_eq!(
        row.try_get::<String, _>("name").unwrap(),
        "messages_conv_created_idx"
    );
}

#[tokio::test]
async fn purge_temporary_run_removes_only_this_turns_rows() {
    let storage = temp_storage().await;
    let pool = storage.pool();
    insert_conversation(pool, "conv-temp", "Temp").await;
    // A prior turn in the same thread must survive the purge.
    insert_message(
        pool,
        "m-prior",
        "conv-temp",
        serde_json::json!({
            "role": "user",
            "content": "earlier",
            "runId": "run-prior",
            "kind": "user",
        }),
    )
    .await;
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES ('run-temp', ?, '{}', 'conv-temp', 'completed', 'rk-temp')",
    )
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert run");
    insert_message(
        pool,
        "m-temp-user",
        "conv-temp",
        serde_json::json!({
            "role": "user",
            "content": "secret",
            "runId": "run-temp",
            "kind": "user",
        }),
    )
    .await;
    insert_message(
        pool,
        "m-temp-assistant",
        "conv-temp",
        serde_json::json!({
            "role": "assistant",
            "content": "answer",
            "runId": "run-temp",
            "kind": "assistant",
        }),
    )
    .await;

    purge_temporary_run(pool, "conv-temp", "run-temp", false)
        .await
        .expect("purge");

    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE id = 'run-temp'")
        .fetch_one(pool)
        .await
        .expect("count runs");
    assert_eq!(runs, 0, "the temporary run row goes");

    let remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM messages ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("list messages");
    assert_eq!(
        remaining,
        vec!["m-prior".to_string()],
        "only the prior turn survives"
    );

    let conversations: i64 = sqlx::query_scalar("SELECT count(*) FROM conversations")
        .fetch_one(pool)
        .await
        .expect("count conversations");
    assert_eq!(conversations, 1, "an existing thread is kept");
}

#[tokio::test]
async fn purge_temporary_run_deletes_a_thread_it_created() {
    let storage = temp_storage().await;
    let pool = storage.pool();
    insert_conversation(pool, "conv-only", "Fresh").await;
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES ('run-only', ?, '{}', 'conv-only', 'completed', 'rk-only')",
    )
    .bind(SINGLE_USER_ID)
    .execute(pool)
    .await
    .expect("insert run");
    insert_message(
        pool,
        "m-only",
        "conv-only",
        serde_json::json!({
            "role": "user",
            "content": "hi",
            "runId": "run-only",
            "kind": "user",
        }),
    )
    .await;

    purge_temporary_run(pool, "conv-only", "run-only", true)
        .await
        .expect("purge");

    let conversations: i64 = sqlx::query_scalar("SELECT count(*) FROM conversations")
        .fetch_one(pool)
        .await
        .expect("count conversations");
    assert_eq!(conversations, 0, "a thread created for the turn is removed");
}
