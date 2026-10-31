//! End-to-end encrypted backup and restore against a real database.

use mixd_storage::{backup, Storage};
use std::env;
use uuid::Uuid;

async fn seeded() -> (std::path::PathBuf, Storage) {
    let dir = env::temp_dir().join(format!("mixd-backup-test-{}", Uuid::new_v4()));
    let storage = Storage::open(&dir).await.expect("open");

    // A conversation is a good sentinel: it exists in the schema and is easy to
    // query back after a restore.
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?1, ?2, ?3)")
        .bind("11111111-1111-1111-1111-111111111111")
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(r#"{"title":"before backup"}"#)
        .execute(storage.pool())
        .await
        .expect("seed conversation");

    (dir, storage)
}

async fn title(storage: &Storage) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT json_extract(data, '$.title') FROM conversations WHERE id = ?",
    )
    .bind("11111111-1111-1111-1111-111111111111")
    .fetch_optional(storage.pool())
    .await
    .ok()
    .flatten()
}

#[tokio::test]
async fn backup_then_restore_round_trips() {
    let (dir, _storage) = seeded().await;
    let dest = dir.join("backup.mixbak");

    let info = backup::create(&dir, &dest, "hunter2-correct")
        .await
        .expect("create backup");
    assert!(info.bytes > 0);
    assert!(
        info.manifest.tables.contains(&"conversations".to_string()),
        "manifest should list the schema, got {:?}",
        info.manifest.tables
    );

    // The archive must not be readable as a SQLite file.
    let raw = tokio::fs::read(&dest).await.expect("read backup");
    assert!(!raw.starts_with(b"SQLite format 3\0"));
    assert!(!raw.windows(10).any(|w| w == b"before backup"));
    assert_eq!(&raw[..8], b"MIXBAK\x01\x00");

    // Change the data, then restore over the top.
    sqlx::query("UPDATE conversations SET data = json_set(data, '$.title', 'changed')")
        .execute(_storage.pool())
        .await
        .expect("mutate");
    drop(_storage);

    let restore = backup::restore(&dir, &dest, "hunter2-correct")
        .await
        .expect("restore");
    assert!(restore
        .manifest
        .tables
        .contains(&"conversations".to_string()));

    // Reopening promotes the staged snapshot.
    let reopened = Storage::open(&dir).await.expect("reopen");
    assert_eq!(
        title(&reopened).await.as_deref(),
        Some("before backup"),
        "restored data should win over the mutated one"
    );
    assert!(!dir.join("mix-agent.restored.db").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wrong_password_does_not_touch_live_data() {
    let (dir, storage) = seeded().await;
    let dest = dir.join("backup.mixbak");
    backup::create(&dir, &dest, "right-password")
        .await
        .expect("create backup");

    let err = backup::restore(&dir, &dest, "wrong-password")
        .await
        .expect_err("must refuse a wrong password");
    assert!(
        err.to_string().contains("wrong password"),
        "unhelpful error: {err}"
    );
    assert!(
        !dir.join("mix-agent.restored.db").exists(),
        "a failed decrypt must not stage anything"
    );
    assert_eq!(title(&storage).await.as_deref(), Some("before backup"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn rejects_files_that_are_not_backups() {
    let dir = env::temp_dir().join(format!("mixd-backup-test-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await.expect("mkdir");
    let bogus = dir.join("bogus.mixbak");
    tokio::fs::write(&bogus, b"this is not a backup at all, honestly")
        .await
        .expect("write");

    let err = backup::restore(&dir, &bogus, "anything")
        .await
        .expect_err("must reject a non-backup");
    assert!(err.to_string().contains("not a MIX-agent backup"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn empty_password_is_refused_on_create() {
    let (dir, _storage) = seeded().await;
    let dest = dir.join("backup.mixbak");
    let err = backup::create(&dir, &dest, "")
        .await
        .expect_err("must require a password");
    assert!(err.to_string().contains("at least 12"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
