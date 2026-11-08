//! The database is currently plaintext (no SQLCipher): 0600 file permissions
//! are the only at-rest protection, so a regression that leaves a
//! world-readable `mix-agent.db` behind must fail loudly here, not in review.

#[cfg(unix)]
use mixd_storage::Storage;

#[cfg(unix)]
#[tokio::test]
async fn database_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!(
        "mix-perms-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let storage = Storage::open(&dir).await.expect("open");
    for name in ["mix-agent.db", "mix-agent.db-wal", "mix-agent.db-shm"] {
        let path = dir.join(name);
        if path.exists() {
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{name} must be owner-only, got {mode:o}");
        }
    }
    drop(storage);
    let _ = std::fs::remove_dir_all(&dir);
}
