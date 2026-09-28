//! SQLite-backed storage layer.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Pool, Sqlite};
use tokio::sync::Mutex;

pub type DbPool = Pool<Sqlite>;

const SCHEMA: &str = include_str!("migrations/0001_initial.sql");

#[derive(Clone)]
pub struct Storage {
    pool: DbPool,
    data_dir: PathBuf,
    write_lock: Arc<Mutex<()>>,
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
        let storage = Self {
            pool,
            data_dir,
            write_lock: Arc::new(Mutex::new(())),
        };
        storage.run_schema().await?;
        Ok(storage)
    }

    /// Synchronous helper for callers that don't have a tokio runtime yet.
    pub fn open_sync(data_dir: impl AsRef<Path>) -> Self {
        let dir = data_dir.as_ref().to_path_buf();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move { Self::open(dir).await.expect("storage open") })
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }
    pub async fn write_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.write_lock.lock().await
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
    let staged = data_dir.join("mix-agent.restored.db");
    if !staged.is_file() {
        return Ok(());
    }
    // WAL sidecars belong to the old database; leaving them would let SQLite
    // replay stale pages over the restored file.
    for sidecar in ["mix-agent.db-wal", "mix-agent.db-shm"] {
        tokio::fs::remove_file(data_dir.join(sidecar)).await.ok();
    }
    let live = data_dir.join("mix-agent.db");
    if live.is_file() {
        tokio::fs::copy(&live, data_dir.join("mix-agent.db.pre-restore"))
            .await
            .ok();
    }
    tokio::fs::rename(&staged, &live)
        .await
        .with_context(|| format!("promoting staged restore into {}", live.display()))?;
    Ok(())
}
