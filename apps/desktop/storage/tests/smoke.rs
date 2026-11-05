use mixd_storage::Storage;
use std::env;

#[tokio::test]
async fn smoke_storage_open() {
    let dir = env::temp_dir().join(format!("mixd-storage-test-{}", uuid::Uuid::new_v4()));
    let storage = Storage::open(&dir).await.expect("open");
    let pool = storage.pool();
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .expect("count users");
    assert_eq!(row.0, 1, "expected the singleton user row");
}

/// Reopening an up-to-date database must not copy the whole file. The
/// synchronous snapshot is reserved for a pending migration; the everyday one
/// is taken later by `snapshot_if_due`, once per day.
#[tokio::test]
async fn reopening_does_not_take_a_database_snapshot() {
    let dir = env::temp_dir().join(format!("mixd-snapshot-test-{}", uuid::Uuid::new_v4()));
    {
        let storage = Storage::open(&dir).await.expect("open");
        sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES ('c1','local','{}')")
            .execute(storage.pool())
            .await
            .expect("conversation");
    }
    let storage = Storage::open(&dir).await.expect("reopen");
    let mut entries = tokio::fs::read_dir(&dir).await.expect("read dir");
    let mut backups = 0;
    while let Some(entry) = entries.next_entry().await.expect("entry") {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("mix-agent.db.backup-")
        {
            backups += 1;
        }
    }
    assert_eq!(backups, 0, "an up-to-date reopen must not snapshot");

    // The background path still creates one, and the marker stops a second
    // snapshot within the interval.
    storage.snapshot_if_due().await;
    storage.snapshot_if_due().await;
    let mut entries = tokio::fs::read_dir(&dir).await.expect("read dir");
    let mut backups = 0;
    while let Some(entry) = entries.next_entry().await.expect("entry") {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("mix-agent.db.backup-")
        {
            backups += 1;
        }
    }
    assert_eq!(backups, 1, "one daily snapshot, not one per call");
}

/// Housekeeping must bound the two tables that grow with every Run, while
/// leaving recent Runs (and the transcript) untouched.
#[tokio::test]
async fn maintenance_prunes_only_old_terminal_run_detail() {
    let dir = env::temp_dir().join(format!("mixd-prune-test-{}", uuid::Uuid::new_v4()));
    let storage = Storage::open(&dir).await.expect("open");
    let pool = storage.pool();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES ('c1','local','{}')")
        .execute(pool)
        .await
        .expect("conversation");
    for (id, created) in [
        ("old", "2020-01-01T00:00:00.000Z"),
        ("new", "2999-01-01T00:00:00.000Z"),
    ] {
        sqlx::query(
            "INSERT INTO runs (id, owner_id, conversation_id, status, request_key, created_at) \
             VALUES (?, 'local', 'c1', 'completed', ?, ?)",
        )
        .bind(id)
        .bind(format!("key-{id}"))
        .bind(created)
        .execute(pool)
        .await
        .expect("run");
        sqlx::query(
            "INSERT INTO run_events (id, run_id, sequence, kind, data) VALUES (?, ?, 1, 'text', '{}')",
        )
        .bind(format!("e-{id}"))
        .bind(id)
        .execute(pool)
        .await
        .expect("event");
        sqlx::query(
            "INSERT INTO run_checkpoints (id, owner_id, run_id, step) VALUES (?, 'local', ?, 1)",
        )
        .bind(format!("chk-{id}"))
        .bind(id)
        .execute(pool)
        .await
        .expect("checkpoint");
    }

    storage.maintenance().await;

    let old_events: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM run_events WHERE run_id = 'old'")
        .fetch_one(pool)
        .await
        .expect("count old events");
    let new_events: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM run_events WHERE run_id = 'new'")
        .fetch_one(pool)
        .await
        .expect("count new events");
    let old_checks: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM run_checkpoints WHERE run_id = 'old'")
            .fetch_one(pool)
            .await
            .expect("count old checkpoints");
    assert_eq!(old_events.0, 0);
    assert_eq!(old_checks.0, 0);
    assert_eq!(new_events.0, 1);
}

#[tokio::test]
async fn smoke_storage_reopen_migrations_idempotent() {
    let dir = env::temp_dir().join(format!("mixd-storage-test-{}", uuid::Uuid::new_v4()));
    {
        let _ = Storage::open(&dir).await.expect("open");
    }
    // Re-open should not error even though the migrations would have been
    // already applied.
    let _ = Storage::open(&dir).await.expect("reopen");
}

/// Transcript rows are mirrored into FTS5 and respond to Japanese substring
/// queries (trigram), updates, and deletes.
#[tokio::test]
async fn message_search_is_populated_and_kept_current() {
    let dir = env::temp_dir().join(format!("mixd-fts-test-{}", uuid::Uuid::new_v4()));
    let storage = Storage::open(&dir).await.expect("open");
    let pool = storage.pool();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES ('c1','local','{}')")
        .execute(pool)
        .await
        .expect("conversation");
    sqlx::query(
        "INSERT INTO messages (id, owner_id, conversation_id, data) VALUES (?, 'local', 'c1', ?)",
    )
    .bind("m1")
    .bind(r#"{"role":"user","content":"こんにちは世界、refactor this function"}"#)
    .execute(pool)
    .await
    .expect("message");

    let hits: Vec<(String,)> = sqlx::query_as(
        "SELECT message_id FROM message_search WHERE message_search MATCH '\"んにち\"'",
    )
    .fetch_all(pool)
    .await
    .expect("Japanese substring search");
    assert_eq!(hits.len(), 1, "trigram must match a CJK substring");
    assert_eq!(hits[0].0, "m1");

    let hits: Vec<(String,)> =
        sqlx::query_as("SELECT message_id FROM message_search WHERE message_search MATCH 'factor'")
            .fetch_all(pool)
            .await
            .expect("English substring search");
    assert_eq!(hits.len(), 1, "trigram must match an English substring");

    sqlx::query("UPDATE messages SET data = ? WHERE id = 'm1'")
        .bind(r#"{"role":"user","content":"replacement text"}"#)
        .execute(pool)
        .await
        .expect("update message");
    let hits: Vec<(String,)> = sqlx::query_as(
        "SELECT message_id FROM message_search WHERE message_search MATCH 'replacement'",
    )
    .fetch_all(pool)
    .await
    .expect("updated search");
    assert_eq!(hits.len(), 1, "updates must reindex the message");

    sqlx::query("DELETE FROM messages WHERE id = 'm1'")
        .execute(pool)
        .await
        .expect("delete message");
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM message_search WHERE message_search MATCH 'replacement'",
    )
    .fetch_one(pool)
    .await
    .expect("search after delete");
    assert_eq!(count.0, 0, "deletes must remove the index row");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Knowledge chunks are mirrored into FTS5 as well; the retrieval path can
/// then ask SQLite for candidates instead of streaming the whole table.
#[tokio::test]
async fn knowledge_search_is_populated_and_kept_current() {
    let dir = env::temp_dir().join(format!("mixd-fts-knowledge-{}", uuid::Uuid::new_v4()));
    let storage = Storage::open(&dir).await.expect("open");
    let pool = storage.pool();
    sqlx::query("INSERT INTO knowledge (id, owner_id, data) VALUES ('k1','local',?)")
        .bind(r#"{"docId":"d1","filename":"notes.md","text":"ローカルエージェントの設計メモ"}"#)
        .execute(pool)
        .await
        .expect("knowledge row");
    let hits: Vec<(String, String)> = sqlx::query_as(
        "SELECT knowledge_id, filename FROM knowledge_search \
         WHERE knowledge_search MATCH '\"エージェント\"'",
    )
    .fetch_all(pool)
    .await
    .expect("knowledge search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "k1");
    assert_eq!(hits[0].1, "notes.md");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A project row written before the folder-first refactor (no `rootPath`)
/// must come back from a reopen rooted at a generated folder, so no project
/// is ever left without somewhere to work.
#[tokio::test]
async fn a_legacy_project_without_root_gets_a_generated_folder() {
    let dir = env::temp_dir().join(format!("mixd-migrate-test-{}", uuid::Uuid::new_v4()));
    let projects_root = env::temp_dir().join(format!("mixd-migrate-root-{}", uuid::Uuid::new_v4()));
    // Keep the generated folder inside the test sandbox instead of the real
    // `~/Documents`.
    env::set_var("MIXD_PROJECTS_ROOT", &projects_root);

    let storage = Storage::open(&dir).await.expect("open");
    sqlx::query("INSERT INTO projects (id, owner_id, data) VALUES (?, ?, ?)")
        .bind("legacy-1")
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(r#"{"name":"Legacy","instructions":"","sources":[],"updatedAt":"2026-01-01T00:00:00Z"}"#)
        .execute(storage.pool())
        .await
        .expect("insert legacy project");
    drop(storage);

    let storage = Storage::open(&dir).await.expect("reopen");
    let row: (String,) = sqlx::query_as(
        "SELECT json_extract(data, '$.rootPath') FROM projects WHERE id = 'legacy-1'",
    )
    .fetch_one(storage.pool())
    .await
    .expect("read rootPath");
    assert!(
        !row.0.is_empty(),
        "rootPath must be assigned by the migration"
    );
    assert!(
        std::path::Path::new(&row.0).is_dir(),
        "the generated folder must exist: {}",
        row.0
    );
    assert!(
        std::path::Path::new(&row.0).starts_with(&projects_root),
        "generated under the configured projects root: {}",
        row.0
    );

    // Re-running must not move or duplicate the folder.
    drop(storage);
    let storage = Storage::open(&dir).await.expect("second reopen");
    let again: (String,) = sqlx::query_as(
        "SELECT json_extract(data, '$.rootPath') FROM projects WHERE id = 'legacy-1'",
    )
    .fetch_one(storage.pool())
    .await
    .expect("read rootPath");
    assert_eq!(again.0, row.0, "the assignment must be stable");

    env::remove_var("MIXD_PROJECTS_ROOT");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&projects_root);
}
