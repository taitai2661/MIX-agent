"""Indexes for conversation listing and full-text-ish search.

The sidebar now filters in PostgreSQL instead of loading every conversation
(and every message) into Python.  These indexes cover the JSONB fields used by
that query and the trigram search over message content.

PostgreSQL-only, like the rest of the Docker edition.  ``IF NOT EXISTS`` keeps
the migration re-runnable after a partial deployment.
"""

from alembic import op

revision = "0026"
down_revision = "0025"


def upgrade():
    op.execute("CREATE EXTENSION IF NOT EXISTS pg_trgm")
    # Conversation filters.
    op.execute(
        "CREATE INDEX IF NOT EXISTS conversations_owner_deleted_archived "
        "ON conversations (owner_id, (data->>'deleted_at'), (data->>'archived_at'))"
    )
    op.execute(
        "CREATE INDEX IF NOT EXISTS conversations_owner_folder "
        "ON conversations (owner_id, (data->>'folder_id'))"
    )
    op.execute(
        "CREATE INDEX IF NOT EXISTS conversations_owner_project "
        "ON conversations (owner_id, (data->>'project_id'))"
    )
    op.execute(
        "CREATE INDEX IF NOT EXISTS conversations_owner_last_message "
        "ON conversations (owner_id, (data->>'last_message_at'))"
    )
    # Message search and chronological reads.
    op.execute(
        "CREATE INDEX IF NOT EXISTS messages_content_trgm "
        "ON messages USING gin ((data->>'content') gin_trgm_ops)"
    )
    op.execute(
        "CREATE INDEX IF NOT EXISTS messages_conversation_created ON messages (conversation_id, created_at)"
    )
    # Bounded retention for terminal run events.
    op.execute("CREATE INDEX IF NOT EXISTS run_events_created ON run_events (created_at)")
    # Read-only audit screen.
    op.execute("CREATE INDEX IF NOT EXISTS audit_events_owner_created ON audit_events (owner_id, created_at)")
    # Expired-session cleanup runs from the scheduler tick.
    op.execute("CREATE INDEX IF NOT EXISTS sessions_expires ON sessions (expires)")


def downgrade():
    for name in (
        "sessions_expires",
        "audit_events_owner_created",
        "run_events_created",
        "messages_conversation_created",
        "messages_content_trgm",
        "conversations_owner_last_message",
        "conversations_owner_project",
        "conversations_owner_folder",
        "conversations_owner_deleted_archived",
    ):
        op.execute(f"DROP INDEX IF EXISTS {name}")
