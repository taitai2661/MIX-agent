-- Cursor pagination for the thread sidebar.
--
-- `list_conversations_page` orders by `(updated_at DESC, id DESC)` and seeks
-- with a `(updated_at, id)` cursor. Without a matching index SQLite sorts the
-- whole table on every page, which is exactly the cost pagination exists to
-- avoid.
CREATE INDEX IF NOT EXISTS conversations_updated_id_idx
    ON conversations(updated_at DESC, id DESC);
