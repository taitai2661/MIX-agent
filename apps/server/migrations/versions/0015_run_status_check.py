"""Add a CHECK constraint so runs.status only ever holds known statuses.

``0001_initial`` creates tables from the current ORM metadata, so fresh
installations already have this constraint and the upgrade path below is
guarded to be a no-op for them.  Existing databases that contain an unknown
``runs.status`` value fail explicitly instead of being silently rewritten.
"""

import sqlalchemy as sa
from alembic import op

revision = "0015"
down_revision = "0014"

CONSTRAINT = "ck_runs_status_valid"
ALLOWED = (
    "queued",
    "running",
    "waiting_approval",
    "completed",
    "failed",
    "cancelled",
    "interrupted",
)


def _existing_constraints(bind):
    return {constraint["name"] for constraint in sa.inspect(bind).get_check_constraints("runs")}


def _stored_statuses(bind):
    return {
        row[0]
        for row in bind.execute(sa.text("SELECT DISTINCT status FROM runs"))
    }


def upgrade():
    bind = op.get_bind()
    if CONSTRAINT in _existing_constraints(bind):
        return
    unknown = _stored_statuses(bind) - set(ALLOWED)
    if unknown:
        raise RuntimeError(
            "runs.status contains unknown value(s); refusing to add constraint: "
            + ", ".join(sorted(str(value) for value in unknown))
        )
    op.create_check_constraint(
        CONSTRAINT,
        "runs",
        "status IN ('queued', 'running', 'waiting_approval', 'completed', 'failed', 'cancelled', 'interrupted')",
    )


def downgrade():
    bind = op.get_bind()
    if CONSTRAINT in _existing_constraints(bind):
        op.drop_constraint(CONSTRAINT, "runs", type_="check")