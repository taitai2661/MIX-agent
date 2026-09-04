"""Add persistent scheduled jobs, their runs, and local notifications."""
from alembic import op
from mix_agent.db.models import ScheduledJob, ScheduledRun, Notification

revision = "0008"
down_revision = "0007"

def upgrade():
    bind = op.get_bind()
    ScheduledJob.__table__.create(bind, checkfirst=True)
    ScheduledRun.__table__.create(bind, checkfirst=True)
    Notification.__table__.create(bind, checkfirst=True)

def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
