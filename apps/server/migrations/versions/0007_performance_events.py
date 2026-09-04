"""Add privacy-minimal completed-answer performance history."""

from alembic import op
from mix_agent.db.models import PerformanceEvent

revision = "0007"
down_revision = "0006"


def upgrade():
    PerformanceEvent.__table__.create(op.get_bind(), checkfirst=True)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
