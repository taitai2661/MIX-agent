"""Store browser Push API subscriptions and their notification cursors."""
from alembic import op

from mix_agent.db.models import PushSubscription

revision = "0016"
down_revision = "0015"


def upgrade():
    PushSubscription.__table__.create(op.get_bind(), checkfirst=True)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
