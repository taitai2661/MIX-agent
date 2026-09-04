"""Initial MIX schema."""

from alembic import op
from mix_agent.db.models import Base

revision = "0001"
down_revision = None


def upgrade():
    Base.metadata.create_all(op.get_bind())
    if op.get_bind().dialect.name == "postgresql":
        op.execute("CREATE EXTENSION IF NOT EXISTS pg_trgm")
        op.execute("CREATE INDEX memory_text_trgm ON memories USING gin ((data->>'content') gin_trgm_ops)")


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
