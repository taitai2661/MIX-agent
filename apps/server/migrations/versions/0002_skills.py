"""Add reusable skill storage."""

from alembic import op
from mix_agent.db.models import Skill, SkillRevision

revision = "0002"
down_revision = "0001"


def upgrade():
    Skill.__table__.create(op.get_bind(), checkfirst=True)
    SkillRevision.__table__.create(op.get_bind(), checkfirst=True)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
