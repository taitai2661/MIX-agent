//! SQLite-backed storage layer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Pool, Sqlite};

pub type DbPool = Pool<Sqlite>;

const SCHEMA: &str = include_str!("migrations/0001_initial.sql");
/// Current schema version, tracked via `PRAGMA user_version`.
/// Bump when adding a new `migrations/00NN_*.sql` step and extend
/// `run_schema` with the corresponding branch.
const SCHEMA_VERSION: i64 = 1;

#[derive(Clone)]
pub struct Storage {
    pool: DbPool,
    data_dir: PathBuf,
}

impl Storage {
    pub async fn open(data_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        tokio::fs::create_dir_all(&data_dir).await?;
        promote_staged_restore(&data_dir).await?;
        let db_path = data_dir.join("mix-agent.db");
        // Best-effort pre-open backup: an interrupted schema/migration must
        // never leave the only copy of the database half-written. Failures
        // only warn — a read-only dir or full disk must not block startup.
        backup_before_open(&data_dir, &db_path).await;
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .context("opening SQLite database")?;
        let storage = Self { pool, data_dir };
        // NOTE: at-rest encryption is not yet wired — `SecretStore::ensure_db_key`
        // stages the key in the OS Keychain, but the SQLite connection above
        // does not apply it (no SQLCipher passphrase). The database file is
        // therefore plaintext; 0o600 permissions below are the only current
        // protection. Wire the passphrase before claiming encryption.
        tracing::warn!("database at-rest encryption is not yet wired; key is staged in Keychain");
        storage.run_schema().await?;
        // Restrict the database files to owner-only. Best-effort for the same
        // reason as the backup: a chmod failure must not block startup.
        restrict_permissions(&storage.data_dir.join("mix-agent.db"));
        restrict_permissions(&storage.data_dir.join("mix-agent.db-wal"));
        restrict_permissions(&storage.data_dir.join("mix-agent.db-shm"));
        crate::migrations::run_data_migrations(storage.pool(), storage.data_dir()).await?;
        Ok(storage)
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    async fn run_schema(&self) -> anyhow::Result<()> {
        // user_version-gated: 0 means a fresh database, SCHEMA_VERSION means
        // current. Future migrations add `else if version < N` branches.
        let mut conn = self.pool.acquire().await.context("acquiring connection")?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *conn)
            .await
            .context("reading PRAGMA user_version")?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }
        if version == 0 {
            for stmt in split_statements(SCHEMA) {
                if stmt.trim().is_empty() {
                    continue;
                }
                sqlx::query(&stmt)
                    .execute(&mut *conn)
                    .await
                    .context("applying schema 0001_initial")?;
            }
        }
        // Future versions: `if version < 2 { apply 0002 ... }`, etc.
        sqlx::query(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
            .execute(&mut *conn)
            .await
            .context("writing PRAGMA user_version")?;
        Ok(())
    }
}

/// Report a best-effort write: failures and no-op updates are logged instead
/// of discarded.
///
/// Discarding these results is what hid a SQL syntax error in
/// `set_run_state`, leaving every run status unpersisted. Callers that need
/// the failure to be fatal should `?` the result themselves.
pub fn log_write(result: Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error>, what: &str) {
    match result {
        Ok(res) if res.rows_affected() == 0 => {
            tracing::warn!(what, "write matched no rows");
        }
        Ok(_) => {}
        Err(err) => tracing::error!(what, %err, "write failed"),
    }
}

fn split_statements(schema: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in schema.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") {
            continue;
        }
        current.push_str(line);
        current.push('\n');
        if trimmed.ends_with(';') {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// Copy the existing database to `mix-agent.db.backup-<timestamp>` before the
/// pool is opened, so an interrupted schema/migration step can be recovered
/// by hand. Best-effort: any failure only warns and startup continues.
async fn backup_before_open(data_dir: &Path, db_path: &Path) {
    if !db_path.is_file() {
        return;
    }
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let dest = data_dir.join(format!("mix-agent.db.backup-{ts}"));
    if let Err(err) = tokio::fs::copy(db_path, &dest).await {
        tracing::warn!(%err, from = %db_path.display(), to = %dest.display(), "pre-open database backup failed; continuing");
    } else {
        restrict_permissions(&dest);
    }
}

/// Restrict a database file to owner-only access (Unix 0o600). Best-effort:
/// failures only warn, since the database itself is already open at this
/// point and a chmod failure must not abort startup.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(_) => {
                if let Err(err) =
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                {
                    tracing::warn!(%err, path = %path.display(), "could not set 0600 on database file");
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "could not stat database file for chmod");
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Promote a verified restore staged by [`crate::backup::restore`].
///
/// Restore cannot swap the database out from under a live connection pool, so it
/// stages the snapshot and the swap happens here, before the pool is opened. The
/// previous database is kept as `mix-agent.db.pre-restore` so a bad restore is
/// recoverable by hand.
async fn promote_staged_restore(data_dir: &Path) -> anyhow::Result<()> {
    use tokio::io::AsyncReadExt;
    let staged = data_dir.join("mix-agent.restored.db");
    if !staged.is_file() {
        return Ok(());
    }
    // Verify before swapping: the staged file is only ever produced by a
    // successful `restore`, but promoting a truncated or foreign file would
    // destroy the live database with no way back.
    let mut magic = [0u8; 16];
    let read = {
        let mut file = tokio::fs::File::open(&staged).await?;
        file.read(&mut magic).await?
    };
    if read < 16 || &magic != b"SQLite format 3\0" {
        anyhow::bail!(
            "the staged restore at {} is not a valid SQLite file; refusing to replace the live database",
            staged.display()
        );
    }
    let live = data_dir.join("mix-agent.db");
    if live.is_file() {
        // The safety copy is what makes a bad restore recoverable by hand, so a
        // failure to write it must abort the promotion rather than silently
        // overwrite the only copy of the live database.
        let safety = data_dir.join("mix-agent.db.pre-restore");
        tokio::fs::copy(&live, &safety).await.with_context(|| {
            format!(
                "copying {} to {} before promoting the restore",
                live.display(),
                safety.display()
            )
        })?;
    }
    // WAL sidecars belong to the old database; leaving them would let SQLite
    // replay stale pages over the restored file. They are copied next to the
    // safety copy first, so the uncheckpointed commits they hold are not lost
    // from it too.
    for (sidecar, kept) in [
        ("mix-agent.db-wal", "mix-agent.db.pre-restore-wal"),
        ("mix-agent.db-shm", "mix-agent.db.pre-restore-shm"),
    ] {
        let from = data_dir.join(sidecar);
        if from.is_file() {
            let kept = data_dir.join(kept);
            tokio::fs::copy(&from, &kept).await.with_context(|| {
                format!(
                    "copying {} to {} before promoting the restore",
                    from.display(),
                    kept.display()
                )
            })?;
        }
        // A missing sidecar is normal; any other failure to remove one is not,
        // because a stale WAL against the restored database loses data.
        if let Err(err) = tokio::fs::remove_file(&from).await {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err).with_context(|| {
                    format!(
                        "removing stale sidecar {} before promoting the restore",
                        from.display()
                    )
                });
            }
        }
    }
    tokio::fs::rename(&staged, &live)
        .await
        .with_context(|| format!("promoting staged restore into {}", live.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `run_schema` stamps `user_version = SCHEMA_VERSION` and a second
    /// `open` on the same directory is idempotent (no error, version stays).
    #[tokio::test]
    async fn schema_version_is_stamped_and_reopen_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("mixd-schema-test-{}", uuid::Uuid::new_v4()));
        let storage = Storage::open(&dir).await.expect("first open");
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(storage.pool())
            .await
            .expect("read user_version");
        assert_eq!(version, SCHEMA_VERSION);
        drop(storage);
        let reopened = Storage::open(&dir)
            .await
            .expect("second open is idempotent");
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(reopened.pool())
            .await
            .expect("read user_version after reopen");
        assert_eq!(version, SCHEMA_VERSION);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
