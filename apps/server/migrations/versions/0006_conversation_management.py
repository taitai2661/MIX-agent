"""Add one-level conversation folders."""

from alembic import op
from mix_agent.db.models import ConversationFolder

revision = "0006"
down_revision = "0005"


def upgrade():
    ConversationFolder.__table__.create(op.get_bind(), checkfirst=True)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
