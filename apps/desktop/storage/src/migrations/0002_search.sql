-- Full-text indexes (schema version 2).
--
-- The README promised FTS5 long before the query paths used it: knowledge and
-- transcript search used to stream every row into Rust. These tables mirror
-- the JSON document columns into an FTS5 index and keep themselves current
-- with triggers, so new writes are searchable without a background job.
--
-- `trigram` is deliberate: `unicode61` treats an entire Japanese sentence as
-- one token, while trigram supports substring matching for CJK. Queries
-- shorter than three characters fall back to the lexical scorer in
-- `core/src/retrieval.rs` because trigram cannot index them.

CREATE VIRTUAL TABLE IF NOT EXISTS message_search USING fts5(
    content,
    message_id UNINDEXED,
    conversation_id UNINDEXED,
    role UNINDEXED,
    tokenize = 'trigram'
);

CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_search USING fts5(
    text,
    knowledge_id UNINDEXED,
    doc_id UNINDEXED,
    filename UNINDEXED,
    tokenize = 'trigram'
);

-- Generated columns make the hot list queries indexable. They are virtual, so
-- the JSON document stays the single source of truth and old rows need no
-- backfill. `list_conversations` used to sort every row by a `json_extract`
-- expression with no index, and count/preview each thread with two correlated
-- subqueries.
--
-- The `ALTER TABLE ... ADD COLUMN` statements live in `db.rs`
-- (`SEARCH_COLUMNS` + `ensure_search_columns`) instead of here: SQLite has no
-- `ADD COLUMN IF NOT EXISTS`, and a database stamped by a different migration
-- lineage can be missing these columns even though its `user_version` is past
-- this step. Rust probes each column before adding it, so this file (which is
-- otherwise idempotent) can be replayed safely.

CREATE INDEX IF NOT EXISTS conversations_updated_idx
    ON conversations(updated_at DESC);
CREATE INDEX IF NOT EXISTS messages_conv_role_idx
    ON messages(conversation_id, role);

-- Backfill existing rows. The whole file runs in one transaction, so a crash
-- cannot leave a half-built index behind. The `NOT EXISTS` guards make a
-- replay (a database whose objects predate this step only partially) a no-op
-- for rows that are already indexed instead of duplicating them.
INSERT INTO message_search(content, message_id, conversation_id, role)
SELECT json_extract(m.data, '$.content'), m.id, m.conversation_id,
       json_extract(m.data, '$.role')
FROM messages m
WHERE json_extract(m.data, '$.role') IN ('user', 'assistant')
  AND COALESCE(json_extract(m.data, '$.content'), '') <> ''
  AND NOT EXISTS (SELECT 1 FROM message_search s WHERE s.message_id = m.id);

INSERT INTO knowledge_search(text, knowledge_id, doc_id, filename)
SELECT json_extract(k.data, '$.text'), k.id,
       COALESCE(json_extract(k.data, '$.docId'), ''),
       COALESCE(json_extract(k.data, '$.filename'), 'document')
FROM knowledge k
WHERE COALESCE(json_extract(k.data, '$.text'), '') <> ''
  AND NOT EXISTS (SELECT 1 FROM knowledge_search s WHERE s.knowledge_id = k.id);

CREATE TRIGGER IF NOT EXISTS messages_search_ai
AFTER INSERT ON messages
BEGIN
    INSERT INTO message_search(content, message_id, conversation_id, role)
    SELECT json_extract(new.data, '$.content'), new.id, new.conversation_id,
           json_extract(new.data, '$.role')
    WHERE json_extract(new.data, '$.role') IN ('user', 'assistant')
      AND COALESCE(json_extract(new.data, '$.content'), '') <> '';
END;

CREATE TRIGGER IF NOT EXISTS messages_search_au
AFTER UPDATE OF data ON messages
BEGIN
    DELETE FROM message_search WHERE message_id = old.id;
    INSERT INTO message_search(content, message_id, conversation_id, role)
    SELECT json_extract(new.data, '$.content'), new.id, new.conversation_id,
           json_extract(new.data, '$.role')
    WHERE json_extract(new.data, '$.role') IN ('user', 'assistant')
      AND COALESCE(json_extract(new.data, '$.content'), '') <> '';
END;

CREATE TRIGGER IF NOT EXISTS messages_search_ad
AFTER DELETE ON messages
BEGIN
    DELETE FROM message_search WHERE message_id = old.id;
END;

CREATE TRIGGER IF NOT EXISTS knowledge_search_ai
AFTER INSERT ON knowledge
BEGIN
    INSERT INTO knowledge_search(text, knowledge_id, doc_id, filename)
    SELECT json_extract(new.data, '$.text'), new.id,
           COALESCE(json_extract(new.data, '$.docId'), ''),
           COALESCE(json_extract(new.data, '$.filename'), 'document')
    WHERE COALESCE(json_extract(new.data, '$.text'), '') <> '';
END;

CREATE TRIGGER IF NOT EXISTS knowledge_search_au
AFTER UPDATE OF data ON knowledge
BEGIN
    DELETE FROM knowledge_search WHERE knowledge_id = old.id;
    INSERT INTO knowledge_search(text, knowledge_id, doc_id, filename)
    SELECT json_extract(new.data, '$.text'), new.id,
           COALESCE(json_extract(new.data, '$.docId'), ''),
           COALESCE(json_extract(new.data, '$.filename'), 'document')
    WHERE COALESCE(json_extract(new.data, '$.text'), '') <> '';
END;

CREATE TRIGGER IF NOT EXISTS knowledge_search_ad
AFTER DELETE ON knowledge
BEGIN
    DELETE FROM knowledge_search WHERE knowledge_id = old.id;
END;
