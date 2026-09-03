"""Alembic migration for the knowledge store."""

from alembic import op

from mix_agent.db.models import Knowledge

revision = "0011"
down_revision = "0010"


def upgrade():
    Knowledge.__table__.create(op.get_bind(), checkfirst=True)
    if op.get_bind().dialect.name == "postgresql":
        op.execute("CREATE EXTENSION IF NOT EXISTS pg_trgm")
        op.execute("CREATE INDEX IF NOT EXISTS knowledge_text_trgm ON knowledge USING gin ((data->>'content') gin_trgm_ops)")


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
