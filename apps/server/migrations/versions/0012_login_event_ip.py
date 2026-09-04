"""Store login connection metadata in a typed column."""

import sqlalchemy as sa
from alembic import op



revision = "0012"
down_revision = "0011"


def upgrade():
    # ``0001_initial`` creates tables from the current ORM metadata.  Fresh
    # installations may therefore already have this column before this later
    # migration runs; existing upgraded installations do not.  Support both
    # paths so an upgrade is safe and repeatable in either case.
    columns = {column["name"] for column in sa.inspect(op.get_bind()).get_columns("login_events")}
    if "ip" not in columns:
        op.add_column(
            "login_events",
            sa.Column("ip", sa.String(length=255), nullable=False, server_default="unknown"),
        )
        op.alter_column("login_events", "ip", server_default=None)


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
