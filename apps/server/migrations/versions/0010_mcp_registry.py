"""MCP Registry and OAuth state.

Revision ID: 0010
Revises: 0009
"""

from alembic import op
import sqlalchemy as sa

revision = "0010"
down_revision = "0009"
branch_labels = None
depends_on = None


def upgrade():
    op.create_table(
        "mcp_auth_states",
        sa.Column("id", sa.String(length=36), primary_key=True),
        sa.Column("owner_id", sa.String(length=36), nullable=False),
        sa.Column("data", sa.JSON(), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
    )
    op.create_index("ix_mcp_auth_states_owner_id", "mcp_auth_states", ["owner_id"])
    op.create_index("mcp_auth_state_owner_created", "mcp_auth_states", ["owner_id", "created_at"])


def downgrade():
    op.drop_index("mcp_auth_state_owner_created", table_name="mcp_auth_states")
    op.drop_index("ix_mcp_auth_states_owner_id", table_name="mcp_auth_states")
    op.drop_table("mcp_auth_states")
