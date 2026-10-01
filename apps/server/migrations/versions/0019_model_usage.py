"""Add privacy-minimal per-call token usage history for cost reporting."""

from alembic import op

from mix_agent.db.models import ModelUsageEvent

revision = "0019"
down_revision = "0018"


def upgrade():
    ModelUsageEvent.__table__.create(op.get_bind(), checkfirst=True)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
