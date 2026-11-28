"""Persist brute-force attempt counters across restarts and workers.

``attempt_counters`` holds transient rate-limit state only (no user data, no
secrets), so unlike the backup-restore migrations its downgrade is a plain
``DROP TABLE``. Losing those rows merely resets in-flight throttle windows.
"""

import sqlalchemy as sa
from alembic import op

revision = "0029"
down_revision = "0028"


def upgrade():
    # 0001 seeds the schema from the current Base.metadata, so a database
    # upgraded from scratch already owns this table. Keep the chain re-runnable
    # like 0025/0026 instead of failing with DuplicateTable.
    if sa.inspect(op.get_bind()).has_table("attempt_counters"):
        return
    op.create_table(
        "attempt_counters",
        sa.Column("id", sa.String(length=64), primary_key=True),
        sa.Column("count", sa.Integer(), nullable=False, server_default="0"),
        sa.Column("window_start", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
    )
    op.alter_column("attempt_counters", "count", server_default=None)


def downgrade():
    # Safe to drop: the table contains only short-lived brute-force counters.
    op.drop_table("attempt_counters")
