-- Memory full-text index (schema version 4).
--
-- Recall used to stream every `memories` row into Rust and parse its JSON on
-- every Run. The score is still computed by `core/src/retrieval.rs`'s
-- `rank_score`, but the *candidate set* can now come from SQLite: the trigram
-- index supports substring matching for CJK and Latin, and the ranker applies
-- the same lexical gate afterwards.
--
-- Like `knowledge_search`, this is a standalone FTS table kept current by
-- triggers on the JSON document. `title` and `content` are joined so one term
-- can match either field.

CREATE VIRTUAL TABLE IF NOT EXISTS memory_search USING fts5(
    text,
    memory_id UNINDEXED,
    tokenize = 'trigram'
);

-- Rebuild rather than `NOT EXISTS` per row: an FTS table has no index on
-- `memory_id`, so the guarded form was O(n²) on a large install. This file
-- only runs when the index or its triggers are missing, and the whole
-- migration is one transaction, so rebuilding is safe.
DELETE FROM memory_search;
INSERT INTO memory_search(text, memory_id)
SELECT COALESCE(json_extract(m.data, '$.title'), '') || ' ' ||
       COALESCE(json_extract(m.data, '$.content'), ''),
       m.id
FROM memories m
WHERE COALESCE(json_extract(m.data, '$.title'), '') || ' ' ||
      COALESCE(json_extract(m.data, '$.content'), '') <> '';

CREATE TRIGGER IF NOT EXISTS memory_search_ai
AFTER INSERT ON memories
BEGIN
    INSERT INTO memory_search(text, memory_id)
    SELECT COALESCE(json_extract(new.data, '$.title'), '') || ' ' ||
           COALESCE(json_extract(new.data, '$.content'), ''),
           new.id
    WHERE COALESCE(json_extract(new.data, '$.title'), '') || ' ' ||
          COALESCE(json_extract(new.data, '$.content'), '') <> '';
END;

CREATE TRIGGER IF NOT EXISTS memory_search_au
AFTER UPDATE OF data ON memories
BEGIN
    DELETE FROM memory_search WHERE memory_id = old.id;
    INSERT INTO memory_search(text, memory_id)
    SELECT COALESCE(json_extract(new.data, '$.title'), '') || ' ' ||
           COALESCE(json_extract(new.data, '$.content'), ''),
           new.id
    WHERE COALESCE(json_extract(new.data, '$.title'), '') || ' ' ||
          COALESCE(json_extract(new.data, '$.content'), '') <> '';
END;

CREATE TRIGGER IF NOT EXISTS memory_search_ad
AFTER DELETE ON memories
BEGIN
    DELETE FROM memory_search WHERE memory_id = old.id;
END;
