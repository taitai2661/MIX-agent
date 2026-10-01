"""Allow a durable manual browser pause while retaining the active-run guard."""

import sqlalchemy as sa
from alembic import op

revision = "0018"
down_revision = "0017"


def upgrade():
    bind = op.get_bind()
    checks = {item["name"] for item in sa.inspect(bind).get_check_constraints("runs")}
    if "ck_runs_status_valid" in checks:
        op.drop_constraint("ck_runs_status_valid", "runs", type_="check")
    op.create_check_constraint(
        "ck_runs_status_valid", "runs",
        "status IN ('queued', 'running', 'waiting_approval', 'paused', 'completed', 'failed', 'cancelled', 'interrupted')",
    )
    op.drop_index("one_active_run", table_name="runs")
    op.create_index("one_active_run", "runs", ["conversation_id"], unique=True,
                    postgresql_where=sa.text("status IN ('queued', 'running', 'waiting_approval', 'paused')"))


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
