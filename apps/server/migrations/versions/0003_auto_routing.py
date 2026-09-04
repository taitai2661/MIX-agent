"""Add per-message Auto routing feedback."""

from alembic import op
from mix_agent.db.models import Feedback

revision = "0003"
down_revision = "0002"


def upgrade():
    Feedback.__table__.create(op.get_bind(), checkfirst=True)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
