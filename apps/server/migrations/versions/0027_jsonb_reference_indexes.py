"""Index the JSONB references used when a provider is deleted.

``delete_provider`` used to load every model, agent and conversation into
Python and filter there.  The queries now use ``->>``/``#>>`` predicates, so
they need expression indexes; without them PostgreSQL falls back to a sequential
scan of each table for every provider deletion.

PostgreSQL-only, like the rest of the Docker edition.  ``IF NOT EXISTS`` keeps
the migration re-runnable after a partial deployment.
"""

from alembic import op

revision = "0027"
down_revision = "0026"


def upgrade():
    op.execute(
        "CREATE INDEX IF NOT EXISTS models_owner_provider ON models (owner_id, ((data->>'provider_id')))"
    )
    op.execute("CREATE INDEX IF NOT EXISTS agents_owner_model ON agents (owner_id, ((data->>'model_id')))")
    op.execute(
        "CREATE INDEX IF NOT EXISTS conversations_owner_selection_model "
        "ON conversations (owner_id, ((data #>> '{selection,model_id}')))"
    )


def downgrade():
    for name in (
        "conversations_owner_selection_model",
        "agents_owner_model",
        "models_owner_provider",
    ):
        op.execute(f"DROP INDEX IF EXISTS {name}")
