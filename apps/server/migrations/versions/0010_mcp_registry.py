"""MCP Registry and OAuth state.

Revision ID: 0010
Revises: 0009
"""

from alembic import op
import sqlalchemy as sa
from sqlalchemy.dialects.postgresql import JSONB

revision = "0010"
down_revision = "0009"
branch_labels = None
depends_on = None


def upgrade():
    bind = op.get_bind()
    if "mcp_auth_states" not in sa.inspect(bind).get_table_names():
        op.create_table(
            "mcp_auth_states",
            sa.Column("id", sa.String(length=36), primary_key=True),
            sa.Column("owner_id", sa.String(length=36), nullable=False),
            sa.Column("data", JSONB, nullable=False),
            sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        )
    existing_indexes = {index["name"] for index in sa.inspect(bind).get_indexes("mcp_auth_states")}
    if "ix_mcp_auth_states_owner_id" not in existing_indexes:
        op.create_index("ix_mcp_auth_states_owner_id", "mcp_auth_states", ["owner_id"])
    if "mcp_auth_state_owner_created" not in existing_indexes:
        op.create_index("mcp_auth_state_owner_created", "mcp_auth_states", ["owner_id", "created_at"])


def downgrade():
    op.drop_index("mcp_auth_state_owner_created", table_name="mcp_auth_states")
    op.drop_index("ix_mcp_auth_states_owner_id", table_name="mcp_auth_states")
    op.drop_table("mcp_auth_states")
