"""Add budget_extension_pending Run state and per-step RunCheckpoint table.

The new ``budget_extension_pending`` status lets the engine pause a Run while
asking the user whether to extend the agent budget.  RunCheckpoint stores
durable snapshots so a long-running agent can be resumed from a chosen step
instead of restarting from the beginning.
"""

import sqlalchemy as sa
from alembic import op

revision = "0020"
down_revision = "0019"


def upgrade():
    bind = op.get_bind()
    checks = {item["name"] for item in sa.inspect(bind).get_check_constraints("runs")}
    if "ck_runs_status_valid" in checks:
        op.drop_constraint("ck_runs_status_valid", "runs", type_="check")
    op.create_check_constraint(
        "ck_runs_status_valid", "runs",
        "status IN ('queued', 'running', 'waiting_approval', 'paused', "
        "'budget_extension_pending', 'completed', 'failed', 'cancelled', 'interrupted')",
    )
    op.drop_index("one_active_run", table_name="runs")
    op.create_index(
        "one_active_run", "runs", ["conversation_id"], unique=True,
        postgresql_where=sa.text(
            "status IN ('queued', 'running', 'waiting_approval', 'paused', 'budget_extension_pending')"
        ),
    )

    op.create_table(
        "run_checkpoints",
        sa.Column("id", sa.String(length=36), primary_key=True),
        sa.Column("owner_id", sa.String(length=36), nullable=False),
        sa.Column("data", sa.JSON, nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("run_id", sa.String(length=36), sa.ForeignKey("runs.id"), nullable=False),
        sa.Column("step", sa.Integer(), nullable=False, server_default="0"),
        sa.Column("tool_count", sa.Integer(), nullable=False, server_default="0"),
        sa.Column("trigger", sa.String(length=30), nullable=False, server_default="interval"),
    )
    op.create_index("run_checkpoint_run_step", "run_checkpoints", ["run_id", "step"])
    op.create_index("run_checkpoint_run_created", "run_checkpoints", ["run_id", "created_at"])
    op.create_index("ix_run_checkpoints_owner_id", "run_checkpoints", ["owner_id"])


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
