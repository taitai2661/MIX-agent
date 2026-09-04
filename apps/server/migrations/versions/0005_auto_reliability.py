"""Add privacy-minimal Auto availability history."""

from alembic import op
from mix_agent.db.models import AutoReliabilityEvent

revision = "0005"
down_revision = "0004"


def upgrade():
    AutoReliabilityEvent.__table__.create(op.get_bind(), checkfirst=True)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
