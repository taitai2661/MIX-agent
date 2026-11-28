-- Audit log (schema version 8, A5).
--
-- Every Tool call and every approval outcome appends exactly one row here.
-- The table is append-only: the two triggers below abort any UPDATE or
-- DELETE, so history can only grow. Tampering with a single row is detected
-- by `verify_chain` (storage/src/audit.rs), because each row's `entry_hash`
-- covers the previous row's hash (`prev_hash` chain, genesis = 'GENESIS').
--
-- `seq` is the chain order (AUTOINCREMENT so concurrent writers still get a
-- total order). `decision` mirrors the lifecycle values used by
-- `tools/src/execute.rs`: awaiting_approval | succeeded | failed | denied |
-- approved | denied_by_user | expired.
--
-- The whole file is idempotent (`IF NOT EXISTS`), following the same
-- contract as 0005/0006/0007: replaying the step is always safe.

CREATE TABLE IF NOT EXISTS audit_log (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
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
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS audit_log_run_idx ON audit_log(run_id);
CREATE INDEX IF NOT EXISTS audit_log_tool_call_idx ON audit_log(tool_call_id);
CREATE INDEX IF NOT EXISTS audit_log_tool_idx ON audit_log(tool_name);
CREATE INDEX IF NOT EXISTS audit_log_created_idx ON audit_log(created_at);

-- Append-only enforcement. Every UPDATE/DELETE aborts with a clear message
-- instead of silently rewriting history.
CREATE TRIGGER IF NOT EXISTS audit_log_no_update
BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;

CREATE TRIGGER IF NOT EXISTS audit_log_no_delete
BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;

-- A persistent chain head. `verify_chain` compares the end of the streamed
-- chain with this row, so deleting the *tail* of the log (which a forward-only
-- hash walk cannot detect) is reported as tampering. Every append and every
-- prune checkpoint updates it inside the same transaction as the rows.
CREATE TABLE IF NOT EXISTS audit_log_head (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    seq INTEGER NOT NULL,
    entry_hash TEXT NOT NULL
);

-- Retention archive. `audit::prune_old_entries` copies rows older than the
-- configured retention window here, then deletes them from the live table
-- inside one transaction. The archive keeps the chain verifiable from GENESIS
-- and is itself append-only; pruning is the controlled exception to the
-- live-table DELETE trigger, never a normal write path. Each row is archived
-- exactly once (seq strictly greater than the archive's max), so the UNIQUE
-- constraint on `id` is never hit on a second prune.
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
