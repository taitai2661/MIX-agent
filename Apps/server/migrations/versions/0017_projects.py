"""Add owner-scoped projects; existing conversations remain unassigned."""
from alembic import op

from mix_agent.db.models import Project

revision = "0017"
down_revision = "0016"


def upgrade():
    Project.__table__.create(op.get_bind(), checkfirst=True)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
