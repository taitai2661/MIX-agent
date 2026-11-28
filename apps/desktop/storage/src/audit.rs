//! Append-only audit log with a `prev_hash` chain (A5).
//!
//! Every Tool call and every approval outcome appends one row to `audit_log`
//! (see `migrations/0008_audit_log.sql`). Each row's `entry_hash` covers the
//! previous row's hash, so editing any single row breaks every later row's
//! verification — see [`verify_chain`].
//!
//! Writes are best-effort by design: the caller (`mixd-tools`) logs a warning
//! and continues when the append fails, because a stuck Run is worse than a
//! missing audit row. Verification failures, on the other hand, are always
//! surfaced to the caller.
//!
//! Concurrent appends are serialized both in-process (a process-wide mutex)
//! and at the database level (`BEGIN IMMEDIATE` on one connection), so two
//! writers can never read the same chain head. Retention cleanup is a separate,
//! explicit operation ([`prune_old_entries`]); normal writes stay append-only.

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::DbPool;
use crate::records::SINGLE_USER_ID;

/// Genesis marker stored as `prev_hash` of the first row.
pub const GENESIS_HASH: &str = "GENESIS";

/// Decision used on the synthetic anchor row written when every live audit
/// row is archived by [`prune_old_entries`]. It keeps the chain head (and the
/// `prev_hash` link into the archive) intact.
pub const PRUNED_CHECKPOINT_DECISION: &str = "pruned_checkpoint";

/// Serializes appends and prune operations inside this process. `BEGIN
/// IMMEDIATE` on a dedicated connection provides the same guarantee between
/// processes, so both layers are needed.
static AUDIT_WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One row of `audit_log`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: i64,
    pub id: String,
    pub owner_id: String,
    pub run_id: String,
    pub tool_call_id: String,
    pub approval_id: Option<String>,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub decision: String,
    pub prev_hash: String,
    pub entry_hash: String,
    pub created_at: DateTime<Utc>,
}

/// Compute the chain hash for one entry.
///
/// The fields are encoded as a canonical JSON array, in the order below, so
/// `|` inside a command/path/argument can never blur two different rows into
/// the same hash. `owner_id` and `approval_id` are security-relevant and are
/// part of the digest; `approval_id` is `null` when absent (distinct from an
/// empty string).
///
/// The field order is part of the contract: changing it invalidates every
/// existing chain, so do not reorder without a migration.
#[allow(clippy::too_many_arguments)]
pub fn entry_hash(
    prev_hash: &str,
    owner_id: &str,
    id: &str,
    run_id: &str,
    tool_call_id: &str,
    approval_id: Option<&str>,
    tool_name: &str,
    arguments: &str,
    decision: &str,
    created_at: &str,
) -> String {
    let canonical = serde_json::json!([
        prev_hash,
        owner_id,
        id,
        run_id,
        tool_call_id,
        approval_id,
        tool_name,
        arguments,
        decision,
        created_at,
    ]);
    let encoded = serde_json::to_vec(&canonical).expect("canonical audit fields are serializable");
    let mut hasher = Sha256::new();
    hasher.update(&encoded);
    hex::encode(hasher.finalize())
}

/// Append one audit row. Returns the stored entry.
///
/// `arguments` should already be redacted by the caller (the display copy),
/// so secrets never reach this table in plaintext.
#[allow(clippy::too_many_arguments)]
pub async fn append_entry(
    pool: &DbPool,
    owner_id: &str,
    run_id: &str,
    tool_call_id: &str,
    approval_id: Option<&str>,
    tool_name: &str,
    arguments: &serde_json::Value,
    decision: &str,
) -> anyhow::Result<AuditEntry> {
    // One writer at a time reads the head and inserts. The mutex alone would
    // not cover a second process sharing the file, hence BEGIN IMMEDIATE on a
    // single dedicated connection below.
    let _guard = AUDIT_WRITE_LOCK.lock().await;
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
    let result = append_in_transaction(
        &mut conn,
        owner_id,
        run_id,
        tool_call_id,
        approval_id,
        tool_name,
        arguments,
        decision,
    )
    .await;
    match result {
        Ok(entry) => {
            sqlx::query("COMMIT").execute(&mut *conn).await?;
            Ok(entry)
        }
        Err(err) => {
            // Leave no open write transaction on this pooled connection.
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            Err(err)
        }
    }
}

/// Update the persistent chain head. Called inside the same write transaction
/// as the row insert (or the prune checkpoint) so the head can never point at
/// a row that is not actually the chain tail.
async fn set_head(conn: &mut sqlx::SqliteConnection, seq: i64, hash: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO audit_log_head (singleton, seq, entry_hash) VALUES (1, ?1, ?2) \
         ON CONFLICT(singleton) DO UPDATE SET seq = excluded.seq, entry_hash = excluded.entry_hash",
    )
    .bind(seq)
    .bind(hash)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn append_in_transaction(
    conn: &mut sqlx::SqliteConnection,
    owner_id: &str,
    run_id: &str,
    tool_call_id: &str,
    approval_id: Option<&str>,
    tool_name: &str,
    arguments: &serde_json::Value,
    decision: &str,
) -> anyhow::Result<AuditEntry> {
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = Utc::now();
    let created_str = created_at.to_rfc3339();
    let arguments_str = arguments.to_string();

    // Both reads and the insert share `conn`, and BEGIN IMMEDIATE above holds
    // the write lock, so `prev_hash` is the real chain head.
    let prev_hash: String =
        sqlx::query_scalar("SELECT entry_hash FROM audit_log ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or_else(|| GENESIS_HASH.to_string());
    let hash = entry_hash(
        &prev_hash,
        owner_id,
        &id,
        run_id,
        tool_call_id,
        approval_id,
        tool_name,
        &arguments_str,
        decision,
        &created_str,
    );

    // RETURNING removes the follow-up SELECT (which was both racy and an
    // extra round-trip).
    let seq: i64 = sqlx::query_scalar(
        "INSERT INTO audit_log \
         (id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         RETURNING seq",
    )
    .bind(&id)
    .bind(owner_id)
    .bind(run_id)
    .bind(tool_call_id)
    .bind(approval_id)
    .bind(tool_name)
    .bind(&arguments_str)
    .bind(decision)
    .bind(&prev_hash)
    .bind(&hash)
    .bind(&created_str)
    .fetch_one(&mut *conn)
    .await?;

    // Persist the new head inside the same transaction. `verify_chain` compares
    // the streamed chain against this row so truncating the tail is detectable.
    set_head(conn, seq, &hash).await?;

    Ok(AuditEntry {
        seq,
        id,
        owner_id: owner_id.to_string(),
        run_id: run_id.to_string(),
        tool_call_id: tool_call_id.to_string(),
        approval_id: approval_id.map(str::to_string),
        tool_name: tool_name.to_string(),
        arguments: arguments.clone(),
        decision: decision.to_string(),
        prev_hash,
        entry_hash: hash,
        created_at,
    })
}

/// Decoded verification row: every hashed field, in [`VERIFY_COLUMNS`] order.
type AuditRow = (
    i64,            // seq
    String,         // id
    String,         // owner_id
    String,         // run_id
    String,         // tool_call_id
    Option<String>, // approval_id
    String,         // tool_name
    String,         // arguments
    String,         // decision
    String,         // prev_hash
    String,         // entry_hash
    String,         // created_at
);

const VERIFY_COLUMNS: &str =
    "seq, id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at";
const LIVE_ROWS: &str = "SELECT seq, id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at FROM audit_log ORDER BY seq ASC";
const ARCHIVE_ROWS: &str = "SELECT seq, id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at FROM audit_log_archive ORDER BY seq ASC";

/// Verify the whole chain in `seq` order, streaming one row at a time.
///
/// Archived (pruned) rows are verified first from [`GENESIS_HASH`], then the
/// live table continues from the archive's last hash, so pruning does not
/// create an unverifiable gap. Returns the number of verified rows. Fails with
/// the `seq` of the first row whose recomputed hash or `prev_hash` link does
/// not match.
pub async fn verify_chain(pool: &DbPool) -> anyhow::Result<u64> {
    let mut expected_prev = GENESIS_HASH.to_string();
    let mut verified = 0u64;
    if archive_table_exists(pool).await? {
        verified += verify_rows(pool, ARCHIVE_ROWS, &mut expected_prev)
            .await
            .context("verifying archived audit rows")?;
    }
    verified += verify_rows(pool, LIVE_ROWS, &mut expected_prev)
        .await
        .context("verifying live audit rows")?;

    // A forward-only walk cannot notice that the *tail* was cut off: every
    // remaining row still links to its predecessor. Compare against the
    // checkpoint that every append/prune updates in the same transaction.
    let head: Option<(i64, String)> =
        sqlx::query_as("SELECT seq, entry_hash FROM audit_log_head WHERE singleton = 1")
            .fetch_optional(pool)
            .await?;
    if let Some((head_seq, head_hash)) = head {
        if head_hash != expected_prev {
            anyhow::bail!(
                "audit chain broken: tail truncated or altered (verified tail {expected_prev}, recorded head {head_hash})"
            );
        }
        if head_seq != verified as i64 {
            anyhow::bail!("audit chain broken: recorded {head_seq} rows but verified {verified}");
        }
    }
    Ok(verified)
}

async fn archive_table_exists(pool: &DbPool) -> anyhow::Result<bool> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'audit_log_archive'",
    )
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// Stream one table's rows and check them against `expected_prev`, which is
/// advanced to the last verified hash. Memory stays O(1) in the log size.
async fn verify_rows(
    pool: &DbPool,
    query: &str,
    expected_prev: &mut String,
) -> anyhow::Result<u64> {
    let mut rows = sqlx::query_as::<_, AuditRow>(query).fetch(pool);
    let mut verified = 0u64;
    while let Some(row) = rows.try_next().await? {
        let (
            seq,
            id,
            owner_id,
            run_id,
            tool_call_id,
            approval_id,
            tool_name,
            arguments,
            decision,
            prev_hash,
            stored_hash,
            created_at,
        ) = row;
        if prev_hash.as_str() != expected_prev.as_str() {
            anyhow::bail!("audit chain broken at seq {seq}: prev_hash mismatch");
        }
        let recomputed = entry_hash(
            &prev_hash,
            &owner_id,
            &id,
            &run_id,
            &tool_call_id,
            approval_id.as_deref(),
            &tool_name,
            &arguments,
            &decision,
            &created_at,
        );
        if recomputed != stored_hash {
            anyhow::bail!("audit chain broken at seq {seq}: entry_hash mismatch");
        }
        *expected_prev = stored_hash;
        verified += 1;
    }
    Ok(verified)
}

/// Archive-and-delete audit rows older than `retention_days`, inside one
/// transaction. Returns the number of rows moved to `audit_log_archive`.
///
/// This is the *only* supported way to shrink the table: it temporarily drops
/// the DELETE trigger inside the transaction and restores it before commit,
/// so normal writes remain append-only. A synthetic checkpoint row preserves
/// the chain head when no live rows remain. `retention_days == 0` disables
/// pruning (the fail-closed default).
pub async fn prune_old_entries(pool: &DbPool, retention_days: u32) -> anyhow::Result<u64> {
    if retention_days == 0 {
        return Ok(0);
    }
    let cutoff = (Utc::now() - chrono::Duration::days(i64::from(retention_days)))
        .format("%Y-%m-%dT%H:%M:%fZ")
        .to_string();

    let _guard = AUDIT_WRITE_LOCK.lock().await;
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
    let result = prune_in_transaction(&mut conn, &cutoff).await;
    match result {
        Ok(count) => {
            sqlx::query("COMMIT").execute(&mut *conn).await?;
            Ok(count)
        }
        Err(err) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            Err(err)
        }
    }
}

// The archive mirrors the live schema but has no AUTOINCREMENT (seq comes
// from the live table) and its own append-only guards. It is created both by
// migration 0008 and idempotently here for databases upgraded in place.
const CREATE_ARCHIVE_SQL: &str = "
CREATE TABLE IF NOT EXISTS audit_log_archive (
    seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    owner_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    approval_id TEXT,
    tool_name TEXT NOT NULL,
    arguments TEXT NOT NULL DEFAULT '{}',
    decision TEXT NOT NULL,
    prev_hash TEXT NOT NULL,
    entry_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    archived_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS audit_log_archive_created_idx ON audit_log_archive(created_at);
CREATE TRIGGER IF NOT EXISTS audit_log_archive_no_update
BEFORE UPDATE ON audit_log_archive
BEGIN
    SELECT RAISE(ABORT, 'audit_log_archive is append-only');
END;
CREATE TRIGGER IF NOT EXISTS audit_log_archive_no_delete
BEFORE DELETE ON audit_log_archive
BEGIN
    SELECT RAISE(ABORT, 'audit_log_archive is append-only');
END;
";

const RESTORE_NO_DELETE_TRIGGER: &str = "
CREATE TRIGGER IF NOT EXISTS audit_log_no_delete
BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;
";

async fn prune_in_transaction(
    conn: &mut sqlx::SqliteConnection,
    cutoff: &str,
) -> anyhow::Result<u64> {
    sqlx::raw_sql(CREATE_ARCHIVE_SQL)
        .execute(&mut *conn)
        .await?;

    // Copy through the last row older than the cutoff, so the archived prefix
    // stays a valid chain (no gaps) and the live suffix links to its tail.
    let last: Option<(i64, String)> = sqlx::query_as(
        "SELECT seq, entry_hash FROM audit_log WHERE created_at < ? ORDER BY seq DESC LIMIT 1",
    )
    .bind(cutoff)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((last_seq, last_hash)) = last else {
        return Ok(0);
    };

    // Archive only rows that are not already in the archive. Copying the whole
    // prefix again on a second prune violated `audit_log_archive.id`'s UNIQUE
    // constraint and rolled back the transaction, permanently disabling retention.
    let archived_max: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM audit_log_archive")
            .fetch_one(&mut *conn)
            .await?;
    sqlx::query(&format!(
        "INSERT INTO audit_log_archive ({VERIFY_COLUMNS}, archived_at) \
         SELECT {VERIFY_COLUMNS}, strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         FROM audit_log WHERE seq > ?1 AND seq <= ?2 ORDER BY seq ASC"
    ))
    .bind(archived_max)
    .bind(last_seq)
    .execute(&mut *conn)
    .await?;

    // Controlled exception to append-only: scoped to this transaction, only
    // for the archived prefix. Both statements are on `conn` so a failure
    // rolls the guard back with the DELETE.
    sqlx::query("DROP TRIGGER IF EXISTS audit_log_no_delete")
        .execute(&mut *conn)
        .await?;
    let deleted = sqlx::query("DELETE FROM audit_log WHERE seq <= ?1")
        .bind(last_seq)
        .execute(&mut *conn)
        .await?;
    sqlx::raw_sql(RESTORE_NO_DELETE_TRIGGER)
        .execute(&mut *conn)
        .await?;

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(&mut *conn)
        .await?;
    if remaining == 0 {
        // Keep a head row so later appends chain onto the archive instead of
        // restarting at GENESIS.
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = Utc::now().to_rfc3339();
        let hash = entry_hash(
            &last_hash,
            SINGLE_USER_ID,
            &id,
            "audit",
            "prune",
            None,
            "audit",
            "{}",
            PRUNED_CHECKPOINT_DECISION,
            &created_at,
        );
        let checkpoint_seq: i64 = sqlx::query_scalar(
            "INSERT INTO audit_log \
             (id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             RETURNING seq",
        )
        .bind(&id)
        .bind(SINGLE_USER_ID)
        .bind("audit")
        .bind("prune")
        .bind(None::<&str>)
        .bind("audit")
        .bind("{}")
        .bind(PRUNED_CHECKPOINT_DECISION)
        .bind(&last_hash)
        .bind(&hash)
        .bind(&created_at)
        .fetch_one(&mut *conn)
        .await?;
        set_head(conn, checkpoint_seq, &hash).await?;
    }
    Ok(deleted.rows_affected())
}

/// Recent entries for one Run, newest first.
pub async fn entries_for_run(
    pool: &DbPool,
    run_id: &str,
    limit: i64,
) -> anyhow::Result<Vec<AuditEntry>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT seq, id, owner_id, run_id, tool_call_id, approval_id, tool_name, arguments, decision, prev_hash, entry_hash, created_at \
          FROM audit_log WHERE run_id = ?1 ORDER BY seq DESC LIMIT ?2",
    )
    .bind(run_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let seq: i64 = row.try_get("seq")?;
        let id: String = row.try_get("id")?;
        let owner_id: String = row.try_get("owner_id")?;
        let run_id: String = row.try_get("run_id")?;
        let tool_call_id: String = row.try_get("tool_call_id")?;
        let approval_id: Option<String> = row.try_get("approval_id")?;
        let tool_name: String = row.try_get("tool_name")?;
        let arguments_raw: String = row.try_get("arguments")?;
        let decision: String = row.try_get("decision")?;
        let prev_hash: String = row.try_get("prev_hash")?;
        let entry_hash: String = row.try_get("entry_hash")?;
        let created_raw: String = row.try_get("created_at")?;
        let arguments: serde_json::Value =
            serde_json::from_str(&arguments_raw).unwrap_or(serde_json::Value::Null);
        let created_at = created_raw
            .parse::<DateTime<Utc>>()
            .or_else(|_| {
                chrono::DateTime::parse_from_rfc3339(&created_raw).map(|t| t.with_timezone(&Utc))
            })
            .unwrap_or_else(|_| Utc::now());
        out.push(AuditEntry {
            seq,
            id,
            owner_id,
            run_id,
            tool_call_id,
            approval_id,
            tool_name,
            arguments,
            decision,
            prev_hash,
            entry_hash,
            created_at,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hash(approval: Option<&str>, owner: &str) -> String {
        entry_hash(
            "GENESIS", owner, "id", "run", "call", approval, "terminal", "{}", "allow", "t",
        )
    }

    #[test]
    fn hash_is_deterministic_and_sensitive() {
        let a = sample_hash(None, "owner");
        let b = sample_hash(None, "owner");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, sample_hash(Some("approval-1"), "owner"));
        assert_ne!(sample_hash(None, "owner"), sample_hash(None, "other"));
        assert_ne!(sample_hash(None, "owner"), sample_hash(Some(""), "owner"));
        let denied = entry_hash(
            "GENESIS", "owner", "id", "run", "call", None, "terminal", "{}", "deny", "t",
        );
        assert_ne!(a, denied);
    }

    #[test]
    fn hash_delimiter_is_unambiguous() {
        // The old `|`-joined encoding collided for these two rows.
        let a = entry_hash(
            "G",
            "o",
            "id",
            "r",
            "c",
            None,
            "t",
            "{\"x\":\"|y\"}",
            "d",
            "ts",
        );
        let b = entry_hash(
            "G",
            "o",
            "id",
            "r",
            "c|t",
            None,
            "",
            "{\"x\":\"y\"}",
            "d",
            "ts",
        );
        assert_ne!(a, b);
    }
}
