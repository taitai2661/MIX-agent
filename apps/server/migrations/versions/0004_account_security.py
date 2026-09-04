"""Add account login history."""

from alembic import op
from mix_agent.db.models import LoginEvent

revision = "0004"
down_revision = "0003"


def upgrade():
    LoginEvent.__table__.create(op.get_bind(), checkfirst=True)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
