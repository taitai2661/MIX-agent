//! SQLite-backed storage layer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Connection as _, Pool, Sqlite, SqliteConnection};

pub type DbPool = Pool<Sqlite>;

const SCHEMA: &str = include_str!("migrations/0001_initial.sql");
/// Full-text search tables and their triggers (schema version 2).
const SEARCH_SCHEMA: &str = include_str!("migrations/0002_search.sql");
/// Current schema version, tracked via `PRAGMA user_version`.
/// Bump when adding a new `migrations/00NN_*.sql` step and extend
/// `run_schema` with the corresponding branch.
const SCHEMA_VERSION: i64 = 5;

/// Conversation pagination index (schema version 3).
const CONVERSATION_PAGE_SCHEMA: &str = include_str!("migrations/0003_conversation_page.sql");
/// Memory full-text index (schema version 4).
const MEMORY_SEARCH_SCHEMA: &str = include_str!("migrations/0004_memory_search.sql");
/// macOS Spotlight index queue and internal state (schema version 5).
const SPOTLIGHT_SCHEMA: &str = include_str!("migrations/0005_spotlight.sql");

/// Virtual (generated) columns added by schema version 2.
///
/// SQLite has no `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`, and
/// `user_version` cannot be trusted on its own: a database written by a build
/// from a different migration lineage can be stamped *ahead* of
/// [`SCHEMA_VERSION`] while missing these columns (the sidebar then fails with
/// `no such column: c.updated_at`). Each entry is
/// `(table, column, generation expression)`; `ensure_search_columns` probes the
/// catalog before adding one, so replaying the step is safe.
const SEARCH_COLUMNS: &[(&str, &str, &str)] = &[
    (
        "conversations",
        "updated_at",
        "json_extract(data, '$.updatedAt')",
    ),
    ("messages", "role", "json_extract(data, '$.role')"),
    ("messages", "content", "json_extract(data, '$.content')"),
];

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

        // Only take the synchronous `VACUUM INTO` snapshot when a schema or
        // data migration is about to run. It is O(database size), so doing it
        // on every launch made start-up grow with the user's history. The
        // daily snapshot for an up-to-date database runs after the window is
        // visible ([`Storage::snapshot_if_due`]).
        let migration_pending = migration_is_pending(&db_path).await;
        let backup_started = std::time::Instant::now();
        if migration_pending && backup_before_open(&data_dir, &db_path).await {
            mark_backup_done(&data_dir).await;
        }
        tracing::info!(
            target: "startup",
            ms = backup_started.elapsed().as_millis() as u64,
            migration_pending,
            "storage: pre-open backup step"
        );

        let open_started = std::time::Instant::now();
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
        tracing::info!(
            target: "startup",
            ms = open_started.elapsed().as_millis() as u64,
            "storage: sqlite open"
        );

        // A corrupt database otherwise fails later with a confusing error from
        // whichever query happens to hit the damaged page first. The full
        // `quick_check` is O(database size); running it on every launch made
        // start-up slow for long-lived installs. It now runs weekly (or when
        // the marker is missing, which includes the first launch and a restore).
        let check_started = std::time::Instant::now();
        let ran_quick_check = quick_check_is_due(&data_dir).await;
        if ran_quick_check {
            match sqlx::query_scalar::<_, String>("PRAGMA quick_check(1)")
                .fetch_all(&pool)
                .await
            {
                Ok(rows) if rows.iter().all(|row| row == "ok") => {
                    mark_quick_check_done(&data_dir).await;
                }
                Ok(rows) => {
                    let detail = rows.join("; ");
                    tracing::error!(%detail, "SQLite quick_check reported corruption");
                    anyhow::bail!(
                        "the database at {} failed SQLite's integrity check: {detail}. \
                         A pre-open copy named mix-agent.db.backup-* is kept next to it; \
                         restore that copy (or an exported backup) and restart.",
                        db_path.display()
                    );
                }
                Err(err) => {
                    // Some builds disable quick_check; do not fail startup for
                    // a diagnostic the engine cannot run.
                    tracing::warn!(%err, "could not run SQLite quick_check");
                    mark_quick_check_done(&data_dir).await;
                }
            }
        }
        tracing::info!(
            target: "startup",
            ms = check_started.elapsed().as_millis() as u64,
            ran = ran_quick_check,
            "storage: integrity check step"
        );

        let storage = Self { pool, data_dir };
        // NOTE: at-rest encryption is not yet wired — `SecretStore::ensure_db_key`
        // stages the key in the OS Keychain, but the SQLite connection above
        // does not apply it (no SQLCipher passphrase). The database file is
        // therefore plaintext; 0o600 permissions below are the only current
        // protection. Wire the passphrase before claiming encryption.
        tracing::warn!("database at-rest encryption is not yet wired; key is staged in Keychain");
        let schema_started = std::time::Instant::now();
        storage.run_schema().await?;
        tracing::info!(
            target: "startup",
            ms = schema_started.elapsed().as_millis() as u64,
            "storage: schema"
        );
        // Restrict the database files to owner-only. Best-effort for the same
        // reason as the backup: a chmod failure must not block startup.
        restrict_permissions(&storage.data_dir.join("mix-agent.db"));
        restrict_permissions(&storage.data_dir.join("mix-agent.db-wal"));
        restrict_permissions(&storage.data_dir.join("mix-agent.db-shm"));
        let data_started = std::time::Instant::now();
        crate::migrations::run_data_migrations(storage.pool(), storage.data_dir()).await?;
        tracing::info!(
            target: "startup",
            ms = data_started.elapsed().as_millis() as u64,
            "storage: data migrations"
        );
        Ok(storage)
    }

    /// Housekeeping that keeps the database from growing without bound.
    ///
    /// `run_events` is the table that grows fastest (one row per streamed
    /// batch), and a replay only needs the recent window. Events belonging to
    /// terminal Runs older than [`EVENT_RETENTION_DAYS`] are removed; the
    /// transcript itself lives in `messages` and is never touched. Runs in
    /// flight are excluded, so a long-running task cannot lose its log.
    pub async fn maintenance(&self) {
        const EVENT_RETENTION_DAYS: i64 = 180;
        // Match the `strftime('%Y-%m-%dT%H:%M:%fZ','now')` format the schema
        // writes, so the `<` comparison is over comparable strings.
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(EVENT_RETENTION_DAYS))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        match sqlx::query(
            "DELETE FROM run_events WHERE run_id IN (\
               SELECT id FROM runs WHERE created_at < ? \
                 AND status IN ('completed','failed','cancelled','interrupted'))",
        )
        .bind(&cutoff)
        .execute(&self.pool)
        .await
        {
            Ok(result) if result.rows_affected() > 0 => {
                tracing::info!(
                    rows = result.rows_affected(),
                    days = EVENT_RETENTION_DAYS,
                    "pruned old run events"
                );
            }
            Ok(_) => {}
            Err(err) => tracing::warn!(%err, "pruning old run events failed"),
        }
        // Checkpoints are only useful while resuming a recent Run; they were
        // the second-fastest-growing table after `run_events`.
        let _ = sqlx::query(
            "DELETE FROM run_checkpoints WHERE run_id IN (\
               SELECT id FROM runs WHERE created_at < ? \
                 AND status IN ('completed','failed','cancelled','interrupted'))",
        )
        .bind(&cutoff)
        .execute(&self.pool)
        .await;
        // `optimize` updates the planner statistics and can run safely on a
        // live database. `PASSIVE` checkpointing never blocks readers/writers;
        // it just moves committed pages out of the WAL when the moment allows.
        let _ = sqlx::query("PRAGMA optimize").execute(&self.pool).await;
        let _ = sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
            .execute(&self.pool)
            .await;
    }

    /// Take a consistent snapshot after start-up when the last one is older
    /// than [`SNAPSHOT_INTERVAL`]. Best-effort: a failure only warns.
    ///
    /// This is the everyday backup path. `Storage::open` keeps the
    /// synchronous snapshot for the one case that needs it — a pending
    /// migration, where a crash must not leave the only copy half-written.
    pub async fn snapshot_if_due(&self) {
        if !self.data_dir.join("mix-agent.db").is_file() {
            return;
        }
        if last_backup_ms(&self.data_dir)
            .await
            .is_some_and(|at| now_ms().saturating_sub(at) < SNAPSHOT_INTERVAL.as_millis())
        {
            return;
        }
        let started = std::time::Instant::now();
        if snapshot_database(&self.data_dir, &self.pool).await {
            mark_backup_done(&self.data_dir).await;
        }
        tracing::info!(
            target: "startup",
            ms = started.elapsed().as_millis() as u64,
            "storage: background snapshot"
        );
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    async fn run_schema(&self) -> anyhow::Result<()> {
        let mut conn = self.pool.acquire().await.context("acquiring connection")?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *conn)
            .await
            .context("reading PRAGMA user_version")?;
        // `user_version` alone is not a safe gate. It is one integer shared by
        // every migration lineage: a database touched by a build whose
        // migrations were numbered differently can report a version *ahead* of
        // this constant while lacking the objects these steps create. Trusting
        // the number then skips the work and the first query that needs a
        // missing column fails (`no such column: c.updated_at`). Probe the
        // schema itself before concluding there is nothing to do.
        if version >= SCHEMA_VERSION && schema_is_current(&mut conn).await? {
            return Ok(());
        }
        // One transaction for every pending step and the version bump: a
        // failure rolls the whole boot back, so the next start retries rather
        // than inheriting a half-created index (or a version stamp that claims
        // work which never landed).
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .context("beginning schema migration")?;
        let result = async {
            // Gate each step on the objects it owns, not on the version number:
            // that is what makes a foreign-lineage database (or a partially
            // replayed step) heal instead of being skipped.
            if !table_exists(&mut conn, "conversations").await? {
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
            if !search_schema_is_current(&mut conn).await? {
                ensure_search_columns(&mut conn).await?;
                sqlx::raw_sql(SEARCH_SCHEMA)
                    .execute(&mut *conn)
                    .await
                    .context("applying schema 0002_search")?;
            }
            // `CREATE INDEX IF NOT EXISTS`: always safe to replay, and cheap.
            sqlx::raw_sql(CONVERSATION_PAGE_SCHEMA)
                .execute(&mut *conn)
                .await
                .context("applying schema 0003_conversation_page")?;
            if !memory_search_is_current(&mut conn).await? {
                sqlx::raw_sql(MEMORY_SEARCH_SCHEMA)
                    .execute(&mut *conn)
                    .await
                    .context("applying schema 0004_memory_search")?;
            }
            // Every statement is `CREATE ... IF NOT EXISTS`, so replaying the
            // step is safe even when only one of the two tables is missing.
            sqlx::raw_sql(SPOTLIGHT_SCHEMA)
                .execute(&mut *conn)
                .await
                .context("applying schema 0005_spotlight")?;
            // Only ever raise the stamp. Lowering it would invite a build from
            // the lineage that wrote the higher number to replay its own
            // (non-idempotent) backfills on the next launch.
            let stamped = version.max(SCHEMA_VERSION);
            sqlx::query(&format!("PRAGMA user_version = {stamped}"))
                .execute(&mut *conn)
                .await
                .context("writing PRAGMA user_version")?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        match result {
            Ok(()) => {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .context("committing schema migration")?;
            }
            Err(err) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(err);
            }
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

/// Does `name` exist in the schema? Asked from the catalog rather than by
/// catching a missing-table error, so a genuine failure of the probe itself
/// still surfaces instead of being read as "not there yet".
async fn table_exists(conn: &mut SqliteConnection, name: &str) -> anyhow::Result<bool> {
    let found: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') AND name = ?",
    )
    .bind(name)
    .fetch_optional(&mut *conn)
    .await
    .with_context(|| format!("checking whether table {name} exists"))?;
    Ok(found.is_some())
}

/// Is `table.column` already there? `table` is always one of this module's
/// constants, never user input. `table_xinfo` rather than `table_info`:
/// `table_info` omits generated columns, which is exactly what version 2 adds.
async fn column_exists(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
) -> anyhow::Result<bool> {
    let count: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM pragma_table_xinfo('{table}') WHERE name = ?"
    ))
    .bind(column)
    .fetch_one(&mut *conn)
    .await
    .with_context(|| format!("checking whether column {table}.{column} exists"))?;
    Ok(count > 0)
}

/// Is a named object (`trigger`, `index`, …) present?
async fn object_exists(
    conn: &mut SqliteConnection,
    kind: &str,
    name: &str,
) -> anyhow::Result<bool> {
    let found: Option<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = ? AND name = ?")
            .bind(kind)
            .bind(name)
            .fetch_optional(&mut *conn)
            .await
            .with_context(|| format!("checking whether {kind} {name} exists"))?;
    Ok(found.is_some())
}

/// Is schema version 2 (the search index and its generated columns) applied?
///
/// Tables and columns are not enough: a database from another migration
/// lineage can have both while missing the triggers that keep the FTS tables
/// current. That state is worse than "unversioned" because new messages are
/// silently unsearchable, so every object the migration owns is checked.
async fn search_schema_is_current(conn: &mut SqliteConnection) -> anyhow::Result<bool> {
    for table in ["message_search", "knowledge_search"] {
        if !table_exists(conn, table).await? {
            return Ok(false);
        }
    }
    for (table, column, _) in SEARCH_COLUMNS {
        if !column_exists(conn, table, column).await? {
            return Ok(false);
        }
    }
    for (kind, name) in [
        ("index", "conversations_updated_idx"),
        ("index", "messages_conv_role_idx"),
        ("trigger", "messages_search_ai"),
        ("trigger", "messages_search_au"),
        ("trigger", "messages_search_ad"),
        ("trigger", "knowledge_search_ai"),
        ("trigger", "knowledge_search_au"),
        ("trigger", "knowledge_search_ad"),
    ] {
        if !object_exists(conn, kind, name).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Is schema version 4 (the memory FTS table and its triggers) applied?
async fn memory_search_is_current(conn: &mut SqliteConnection) -> anyhow::Result<bool> {
    if !table_exists(conn, "memory_search").await? {
        return Ok(false);
    }
    for name in ["memory_search_ai", "memory_search_au", "memory_search_ad"] {
        if !object_exists(conn, "trigger", name).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Add whichever generated columns version 2 is still missing. Idempotent:
/// each column is probed first because SQLite lacks `ADD COLUMN IF NOT EXISTS`.
async fn ensure_search_columns(conn: &mut SqliteConnection) -> anyhow::Result<()> {
    for (table, column, expression) in SEARCH_COLUMNS {
        if column_exists(conn, table, column).await? {
            continue;
        }
        sqlx::query(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} TEXT GENERATED ALWAYS AS ({expression}) VIRTUAL"
        ))
        .execute(&mut *conn)
        .await
        .with_context(|| format!("adding generated column {table}.{column}"))?;
    }
    Ok(())
}

/// Every object this build expects, regardless of what `user_version` says.
async fn schema_is_current(conn: &mut SqliteConnection) -> anyhow::Result<bool> {
    if !search_schema_is_current(conn).await? {
        return Ok(false);
    }
    if !memory_search_is_current(conn).await? {
        return Ok(false);
    }
    if !table_exists(conn, "spotlight_items").await?
        || !table_exists(conn, "spotlight_meta").await?
    {
        return Ok(false);
    }
    let index: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'index' AND name = 'conversations_updated_id_idx'",
    )
    .fetch_one(&mut *conn)
    .await
    .context("checking for the conversation pagination index")?;
    Ok(index > 0)
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

/// Number of pre-open snapshots kept next to the database.
const BACKUP_RETENTION: usize = 5;

/// How often an up-to-date database is snapshotted after start-up.
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the full `quick_check` runs. Corruption is still detected on the
/// first launch after a restore and whenever the marker is missing.
const QUICK_CHECK_INTERVAL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Marker files live beside the database, not inside it: a database that
/// cannot be read must not also hide the fact that a check is due.
const BACKUP_STATE_FILE: &str = "mix-agent.backup-state";
const QUICK_CHECK_STATE_FILE: &str = "mix-agent.quick-check-state";

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

async fn read_state_ms(data_dir: &Path, name: &str) -> Option<u128> {
    let raw = tokio::fs::read_to_string(data_dir.join(name)).await.ok()?;
    raw.trim().parse::<u128>().ok()
}

async fn write_state_ms(data_dir: &Path, name: &str) {
    let _ = tokio::fs::write(data_dir.join(name), now_ms().to_string()).await;
}

async fn last_backup_ms(data_dir: &Path) -> Option<u128> {
    read_state_ms(data_dir, BACKUP_STATE_FILE).await
}

async fn mark_backup_done(data_dir: &Path) {
    write_state_ms(data_dir, BACKUP_STATE_FILE).await;
}

async fn quick_check_is_due(data_dir: &Path) -> bool {
    last_quick_check_ms(data_dir)
        .await
        .is_none_or(|at| now_ms().saturating_sub(at) >= QUICK_CHECK_INTERVAL.as_millis())
}

async fn last_quick_check_ms(data_dir: &Path) -> Option<u128> {
    read_state_ms(data_dir, QUICK_CHECK_STATE_FILE).await
}

async fn mark_quick_check_done(data_dir: &Path) {
    write_state_ms(data_dir, QUICK_CHECK_STATE_FILE).await;
}

/// Does opening this database run a schema or data migration?
///
/// Asked *before* the pool is opened, because the answer decides whether the
/// blocking `VACUUM INTO` snapshot is needed. A read-only connection is used so
/// a probing failure cannot create or modify the file.
async fn migration_is_pending(db_path: &Path) -> bool {
    if !db_path.is_file() {
        return false;
    }
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false)
        .read_only(true);
    let Ok(mut conn) = SqliteConnection::connect_with(&options).await else {
        // An unreadable database needs the snapshot more than a clean one does.
        return true;
    };
    let version: i64 = match sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut conn)
        .await
    {
        Ok(version) => version,
        Err(err) => {
            tracing::warn!(%err, "could not read PRAGMA user_version; taking a snapshot");
            return true;
        }
    };
    if version < SCHEMA_VERSION {
        return true;
    }
    match schema_is_current(&mut conn).await {
        Ok(current) => !current,
        Err(err) => {
            tracing::warn!(%err, "could not probe the schema; taking a snapshot");
            true
        }
    }
}

/// Snapshot the live database with `VACUUM INTO`. Returns whether it landed.
async fn snapshot_database(data_dir: &Path, pool: &DbPool) -> bool {
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let dest = data_dir.join(format!("mix-agent.db.backup-{ts}"));
    // `VACUUM INTO` cannot take a bind parameter in every SQLite build;
    // escape a local path into a SQL string literal instead. `data_dir` comes
    // from the app, never from user input.
    let literal = format!("'{}'", dest.to_string_lossy().replace('\'', "''"));
    match sqlx::query(&format!("VACUUM INTO {literal}"))
        .execute(pool)
        .await
    {
        Ok(_) => {
            restrict_permissions(&dest);
            prune_backups(data_dir).await;
            true
        }
        Err(err) => {
            tracing::warn!(
                %err,
                to = %dest.display(),
                "background database snapshot failed; continuing"
            );
            false
        }
    }
}

/// Copy the existing database to `mix-agent.db.backup-<timestamp>` before the
/// pool is opened, so an interrupted schema/migration step can be recovered
/// by hand. Best-effort: any failure only warns and startup continues.
///
/// `VACUUM INTO` is deliberate rather than a file copy. The database runs in
/// WAL mode, so committed rows can still live in `-wal`; copying only the main
/// file can lose them, and copying a live file can capture a torn page. The
/// destination is a single consistent snapshot. Older snapshots are pruned so
/// an app launched every day does not accumulate a full copy each time.
async fn backup_before_open(data_dir: &Path, db_path: &Path) -> bool {
    if !db_path.is_file() {
        return true;
    }
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let dest = data_dir.join(format!("mix-agent.db.backup-{ts}"));
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false);
    let snapshot = async {
        let mut conn = SqliteConnection::connect_with(&options).await?;
        // `VACUUM INTO` cannot take a bind parameter in every SQLite build;
        // escape a local path into a SQL string literal instead. `data_dir`
        // comes from the app, never from user input.
        let literal = format!("'{}'", dest.to_string_lossy().replace('\'', "''"));
        sqlx::query(&format!("VACUUM INTO {literal}"))
            .execute(&mut conn)
            .await?;
        Ok::<(), sqlx::Error>(())
    }
    .await;
    let backed_up = match snapshot {
        Ok(()) => {
            restrict_permissions(&dest);
            true
        }
        Err(err) => {
            tracing::warn!(
                %err,
                from = %db_path.display(),
                to = %dest.display(),
                "consistent pre-open database backup failed; falling back to a file copy"
            );
            let _ = tokio::fs::remove_file(&dest).await;
            if let Err(copy_err) = tokio::fs::copy(db_path, &dest).await {
                tracing::warn!(%copy_err, from = %db_path.display(), to = %dest.display(), "pre-open database backup failed; continuing");
                false
            } else {
                // Best effort: a raw copy is only complete when the WAL
                // sidecars come with it. This is the fallback, not the norm.
                for (from, suffix) in [("mix-agent.db-wal", "-wal"), ("mix-agent.db-shm", "-shm")] {
                    let source = data_dir.join(from);
                    if source.is_file() {
                        let target = data_dir.join(format!("mix-agent.db.backup-{ts}{suffix}"));
                        let _ = tokio::fs::copy(&source, &target).await;
                        restrict_permissions(&target);
                    }
                }
                restrict_permissions(&dest);
                true
            }
        }
    };
    prune_backups(data_dir).await;
    backed_up
}

/// Keep only the newest [`BACKUP_RETENTION`] snapshots, including sidecars.
async fn prune_backups(data_dir: &Path) {
    const PREFIX: &str = "mix-agent.db.backup-";
    let Ok(mut entries) = tokio::fs::read_dir(data_dir).await else {
        return;
    };
    let mut stamps: HashSet<String> = HashSet::new();
    let mut files: Vec<(String, String)> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix(PREFIX) else {
            continue;
        };
        let stamp = rest
            .strip_suffix("-wal")
            .or_else(|| rest.strip_suffix("-shm"))
            .unwrap_or(rest)
            .to_string();
        if stamp.is_empty() {
            continue;
        }
        stamps.insert(stamp.clone());
        files.push((name, stamp));
    }
    let mut ordered: Vec<String> = stamps.into_iter().collect();
    // The timestamp is ISO-8601 basic format, so lexical descending order is
    // newest first without parsing.
    ordered.sort_unstable_by(|a, b| b.cmp(a));
    let keep: HashSet<String> = ordered.into_iter().take(BACKUP_RETENTION).collect();
    for (name, stamp) in files {
        if keep.contains(&stamp) {
            continue;
        }
        let _ = tokio::fs::remove_file(data_dir.join(name)).await;
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

#[cfg(test)]
mod upgrade_tests {
    use super::*;

    /// v0.5.5 shipped `user_version = 1` without the search schema. Opening
    /// that database must apply 0002 and backfill existing rows, not just work
    /// for fresh installs.
    #[tokio::test]
    async fn a_version_1_database_is_upgraded_with_a_backfilled_index() {
        let dir = std::env::temp_dir().join(format!("mixd-upgrade-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let db_path = dir.join("mix-agent.db");
        {
            let opts = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();
            for stmt in split_statements(SCHEMA) {
                if !stmt.trim().is_empty() {
                    sqlx::query(&stmt).execute(&pool).await.unwrap();
                }
            }
            sqlx::query(
                "INSERT INTO conversations (id, owner_id, data) VALUES ('c1','local','{}')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO messages (id, owner_id, conversation_id, data) \
                 VALUES ('m1','local','c1','{\"role\":\"user\",\"content\":\"hello upgrade\"}')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("PRAGMA user_version = 1")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
        }

        let storage = Storage::open(&dir).await.expect("upgrade open");
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(storage.pool())
            .await
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let hits: Vec<(String,)> = sqlx::query_as(
            "SELECT message_id FROM message_search WHERE message_search MATCH '\"upgrade\"'",
        )
        .fetch_all(storage.pool())
        .await
        .expect("backfilled index");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "m1");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// A database written by a build with a *different* migration lineage can
    /// report a `user_version` ahead of this crate's constant (the shared dev
    /// database was stamped 6 by an older numbering) while missing the objects
    /// version 2 owns. Opening it has to repair the schema rather than trust
    /// the number — otherwise the sidebar fails with
    /// `no such column: c.updated_at` on every launch.
    #[tokio::test]
    async fn a_higher_user_version_without_the_objects_is_repaired() {
        let dir = std::env::temp_dir().join(format!("mixd-foreign-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let db_path = dir.join("mix-agent.db");
        {
            let opts = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();
            // Only the baseline schema: no search tables, no generated columns.
            for stmt in split_statements(SCHEMA) {
                if !stmt.trim().is_empty() {
                    sqlx::query(&stmt).execute(&pool).await.unwrap();
                }
            }
            sqlx::query(
                "INSERT INTO conversations (id, owner_id, data) \
                 VALUES ('c1','local','{\"title\":\"foreign\",\"updatedAt\":\"2026-01-01T00:00:00Z\"}')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("PRAGMA user_version = 6")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
        }

        let storage = Storage::open(&dir).await.expect("open must repair");
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(storage.pool())
            .await
            .unwrap();
        // Repaired to the objects this build needs, never lowered to 3: a
        // build from the lineage that wrote 6 would otherwise replay its own
        // backfills on the next launch.
        assert_eq!(version, 6);

        // The exact shape the thread sidebar uses.
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT c.id, c.updated_at FROM conversations c \
             ORDER BY c.updated_at DESC, c.id DESC",
        )
        .fetch_all(storage.pool())
        .await
        .expect("sidebar query has its column");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.as_deref(), Some("2026-01-01T00:00:00Z"));

        let search_tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name IN ('message_search', 'knowledge_search')",
        )
        .fetch_one(storage.pool())
        .await
        .unwrap();
        assert_eq!(search_tables, 2);
        drop(storage);

        // A second open sees a current schema and leaves it alone.
        let reopened = Storage::open(&dir).await.expect("reopen is idempotent");
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(reopened.pool())
            .await
            .unwrap();
        assert_eq!(version, 6);
        drop(reopened);

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// Tables and generated columns can exist while a trigger is missing —
    /// the exact state an interrupted/foreign migration leaves. Opening must
    /// replay 0002 rather than trusting that partial shape.
    #[tokio::test]
    async fn a_missing_search_trigger_is_repaired() {
        let dir = std::env::temp_dir().join(format!("mixd-trigger-{}", uuid::Uuid::new_v4()));
        let storage = Storage::open(&dir).await.expect("initial open");
        sqlx::query("DROP TRIGGER messages_search_ai")
            .execute(storage.pool())
            .await
            .expect("drop trigger");
        drop(storage);

        let storage = Storage::open(&dir).await.expect("repair open");
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'trigger' AND name = 'messages_search_ai'",
        )
        .fetch_one(storage.pool())
        .await
        .unwrap();
        assert_eq!(count, 1);
        drop(storage);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// The pre-open backup is a consistency snapshot, not an append-only log:
    /// only the newest few are kept.
    #[tokio::test]
    async fn old_backups_are_pruned() {
        let dir = std::env::temp_dir().join(format!("mixd-backup-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        for stamp in [
            "20260101T000000Z",
            "20260102T000000Z",
            "20260103T000000Z",
            "20260104T000000Z",
            "20260105T000000Z",
            "20260106T000000Z",
            "20260107T000000Z",
        ] {
            tokio::fs::write(dir.join(format!("mix-agent.db.backup-{stamp}")), b"x")
                .await
                .unwrap();
        }
        prune_backups(&dir).await;
        let mut names = Vec::new();
        let mut entries = tokio::fs::read_dir(&dir).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        names.sort();
        assert_eq!(names.len(), BACKUP_RETENTION);
        assert_eq!(
            names.first().map(String::as_str),
            Some("mix-agent.db.backup-20260103T000000Z")
        );
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
