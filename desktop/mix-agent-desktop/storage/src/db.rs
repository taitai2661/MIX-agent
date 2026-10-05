//! SQLite-backed storage layer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Pool, Sqlite};

pub type DbPool = Pool<Sqlite>;

const SCHEMA: &str = include_str!("migrations/0001_initial.sql");

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
        storage.run_schema().await?;
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
        // Idempotent: every statement uses IF NOT EXISTS / OR IGNORE so re-running
        // the schema after a successful first boot is a no-op.
        let mut conn = self.pool.acquire().await.context("acquiring connection")?;
        for stmt in split_statements(SCHEMA) {
            if stmt.trim().is_empty() {
                continue;
            }
            sqlx::query(&stmt)
                .execute(&mut *conn)
                .await
                .context("applying schema statement")?;
        }
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
