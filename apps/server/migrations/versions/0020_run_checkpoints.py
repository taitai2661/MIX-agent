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

    # 0001 creates every current model table on a fresh database, so guard the
    # table and its indexes the same way 0010 does to keep upgrades re-runnable.
    inspector = sa.inspect(bind)
    if "run_checkpoints" not in inspector.get_table_names():
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
    existing_indexes = {index["name"] for index in sa.inspect(bind).get_indexes("run_checkpoints")}
    for name, columns in (
        ("run_checkpoint_run_step", ["run_id", "step"]),
        ("run_checkpoint_run_created", ["run_id", "created_at"]),
        ("ix_run_checkpoints_owner_id", ["owner_id"]),
    ):
        if name not in existing_indexes:
            op.create_index(name, "run_checkpoints", columns)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
