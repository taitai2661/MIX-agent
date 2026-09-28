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
