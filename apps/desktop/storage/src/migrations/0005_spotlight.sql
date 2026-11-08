-- Spotlight index queue and metadata (schema version 5).
--
-- `spotlight_items` is the durable queue *and* the diff baseline: the worker
-- indexes rows in `pending`, deletes rows in `deleted`, and `content_hash`
-- lets a reconcile skip items whose payload did not change. It deliberately
-- has no foreign key to conversations/projects/knowledge: those rows are
-- deleted independently of the index, and a missing source is how the worker
-- detects a stale item.

CREATE TABLE IF NOT EXISTS spotlight_items (
    item_id      TEXT PRIMARY KEY,
    domain       TEXT NOT NULL,
    source_id    TEXT NOT NULL,
    content_hash TEXT NOT NULL DEFAULT '',
    state        TEXT NOT NULL DEFAULT 'pending',
    indexed_at   TEXT,
    last_error   TEXT
);

CREATE INDEX IF NOT EXISTS spotlight_items_state_idx
    ON spotlight_items(state, domain);

-- Internal state that is not a user preference (schema version, last sync,
-- last error). Kept out of `app_settings` because `write_settings` replaces
-- that document wholesale.
CREATE TABLE IF NOT EXISTS spotlight_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
