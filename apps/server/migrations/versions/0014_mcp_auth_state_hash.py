"""Index OAuth state hashes for direct callback lookup."""

import sqlalchemy as sa
from alembic import op

revision = "0014"
down_revision = "0013"


def upgrade():
    # ``0001_initial`` builds tables from current ORM metadata, so fresh
    # installations already have the column; upgraded installations do not.
    columns = {column["name"] for column in sa.inspect(op.get_bind()).get_columns("mcp_auth_states")}
    if "state_hash" not in columns:
        op.add_column(
            "mcp_auth_states",
            sa.Column("state_hash", sa.String(length=64), nullable=False, server_default=""),
        )
        op.alter_column("mcp_auth_states", "state_hash", server_default=None)
    indexes = {index["name"] for index in sa.inspect(op.get_bind()).get_indexes("mcp_auth_states")}
    if "ix_mcp_auth_states_state_hash" not in indexes:
        op.create_index("ix_mcp_auth_states_state_hash", "mcp_auth_states", ["state_hash"])


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
