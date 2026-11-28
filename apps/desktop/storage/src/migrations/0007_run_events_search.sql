-- Run event full-text index (schema version 7).
--
-- `run_events` is the fastest-growing table and the raw material of the
-- Trajectory timeline view. Transcript search (`message_search`) never saw
-- tool calls, streamed text deltas or status transitions, so a trajectory
-- query like "which run touched Cargo.toml" had no index to use. This table
-- mirrors the text-bearing fields of each event's JSON payload into an FTS5
-- index and keeps itself current with triggers, following the same pattern
-- as `0002_search.sql`.
--
-- `trigram` is deliberate for the same reason as the message index:
-- `unicode61` treats an entire Japanese sentence as one token, while trigram
-- supports substring matching for CJK. Queries shorter than three characters
-- fall back to an empty result in `fts_match_query` (`core/src/retrieval.rs`).
--
-- The indexed text is a concatenation of the scalar fields an event kind
-- carries (`delta`, `name`, `error`, `reason`, `hint`, `status`) plus the
-- JSON rendering of the structured payloads (`arguments`, `result`):
-- `json_extract` on an object/array value returns its JSON text, which is
-- exactly what a trajectory search wants to match (a file path inside a tool
-- call, an error body, a checkpoint id).

CREATE VIRTUAL TABLE IF NOT EXISTS run_events_search USING fts5(
    text,
    run_id UNINDEXED,
    sequence UNINDEXED,
    kind UNINDEXED,
    tokenize = 'trigram'
);

-- Backfill existing rows. The whole file runs in one transaction, so a crash
-- cannot leave a half-built index behind. The `NOT EXISTS` guard makes a
-- replay (a database whose objects predate this step only partially) a no-op
-- for rows that are already indexed instead of duplicating them.
INSERT INTO run_events_search(text, run_id, sequence, kind)
SELECT
    COALESCE(json_extract(e.data, '$.delta'), '') || ' ' ||
    COALESCE(json_extract(e.data, '$.name'), '') || ' ' ||
    COALESCE(json_extract(e.data, '$.arguments'), '') || ' ' ||
    COALESCE(json_extract(e.data, '$.result'), '') || ' ' ||
    COALESCE(json_extract(e.data, '$.error'), '') || ' ' ||
    COALESCE(json_extract(e.data, '$.reason'), '') || ' ' ||
    COALESCE(json_extract(e.data, '$.hint'), '') || ' ' ||
    COALESCE(json_extract(e.data, '$.status'), '') || ' ' ||
    COALESCE(json_extract(e.data, '$.checkpoint_id'), ''),
    e.run_id, e.sequence, e.kind
FROM run_events e
WHERE NOT EXISTS (
    SELECT 1 FROM run_events_search s
    WHERE s.run_id = e.run_id AND s.sequence = e.sequence
);

CREATE TRIGGER IF NOT EXISTS run_events_search_ai
AFTER INSERT ON run_events
BEGIN
    INSERT INTO run_events_search(text, run_id, sequence, kind)
    SELECT
        COALESCE(json_extract(new.data, '$.delta'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.name'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.arguments'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.result'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.error'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.reason'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.hint'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.status'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.checkpoint_id'), ''),
        new.run_id, new.sequence, new.kind;
END;

CREATE TRIGGER IF NOT EXISTS run_events_search_au
AFTER UPDATE OF data ON run_events
BEGIN
    DELETE FROM run_events_search WHERE run_id = old.run_id AND sequence = old.sequence;
    INSERT INTO run_events_search(text, run_id, sequence, kind)
    SELECT
        COALESCE(json_extract(new.data, '$.delta'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.name'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.arguments'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.result'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.error'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.reason'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.hint'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.status'), '') || ' ' ||
        COALESCE(json_extract(new.data, '$.checkpoint_id'), ''),
        new.run_id, new.sequence, new.kind;
END;

CREATE TRIGGER IF NOT EXISTS run_events_search_ad
AFTER DELETE ON run_events
BEGIN
    DELETE FROM run_events_search WHERE run_id = old.run_id AND sequence = old.sequence;
END;
