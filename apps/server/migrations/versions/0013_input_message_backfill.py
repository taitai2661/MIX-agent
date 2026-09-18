"""Backfill input_message_id on legacy runs.

Runs created before the field existed were paired to user turns positionally,
which breaks after restore, deletions, or draft edits.  Backfill each run with
the nearest user message created at-or-before it in the same conversation.
"""

from alembic import op

revision = "0013"
down_revision = "0012"


def upgrade():
    op.execute(
        """
        WITH user_messages AS (
            SELECT id, conversation_id, created_at
            FROM messages
            WHERE data->>'role' = 'user'
        ),
        matched AS (
            SELECT DISTINCT ON (r.id)
                r.id AS run_id,
                um.id AS message_id
            FROM runs r
            JOIN user_messages um
                ON um.conversation_id = r.conversation_id
                AND um.created_at <= r.created_at
            WHERE r.data->>'input_message_id' IS NULL
            ORDER BY r.id, um.created_at DESC, um.id DESC
        )
        UPDATE runs r
        SET data = r.data || jsonb_build_object('input_message_id', matched.message_id)
        FROM matched
        WHERE r.id = matched.run_id
        """
    )


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")