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
