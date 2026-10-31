"""Drop the single-column message index covered by the composite one.

``ix_messages_conversation_id`` was created by the implicit ``index=True`` on
``Message.conversation_id``. Migration 0026 added
``messages_conversation_created`` on ``(conversation_id, created_at)``, which
serves every query that filters by conversation, so the single-column index is
now duplicated write and storage work.

PostgreSQL-only, like the rest of the Docker edition. ``IF EXISTS`` keeps this
re-runnable after a partial deployment.
"""

from alembic import op

revision = "0028"
down_revision = "0027"


def upgrade():
    op.execute("DROP INDEX IF EXISTS ix_messages_conversation_id")


def downgrade():
    op.execute("CREATE INDEX IF NOT EXISTS ix_messages_conversation_id ON messages (conversation_id)")
