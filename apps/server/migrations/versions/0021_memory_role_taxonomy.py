"""Role-based Memory: extend the Memory table and add Evidence/Conflict.

This migration widens the ``memories`` table to carry a role-based taxonomy
and adds two first-class concepts:

- ``memory_evidences`` — citations that justify a Memory item.
- ``memory_conflicts`` — disagreements that must be resolved before they
  silently overwrite one another.

Existing rows are backfilled with sensible defaults (``role=fact``,
``scope=user``, ``source_kind=agent``, ``verification=unverified``,
``lifecycle_state`` mapped through ``LEGACY_LIFECYCLE_MAP``) so the new
runtime can read legacy data without crashing.
"""

import sqlalchemy as sa
from alembic import op

revision = "0021"
down_revision = "0020"


LEGACY_LIFECYCLE_MAP = {
    "established": "active",
    "latent": "candidate",
    "superseded": "superseded",
    "archived": "archived",
    "deleted": "expired",
}


def upgrade():
    bind = op.get_bind()

    with op.batch_alter_table("memories") as batch:
        batch.add_column(sa.Column("role", sa.String(length=20), nullable=False, server_default="fact"))
        batch.add_column(sa.Column("scope", sa.String(length=20), nullable=False, server_default="user"))
        batch.add_column(sa.Column("source_kind", sa.String(length=20), nullable=False, server_default="agent"))
        batch.add_column(sa.Column("task_id", sa.String(length=36), nullable=True))
        batch.add_column(sa.Column("superseded_by_id", sa.String(length=36), nullable=True))
        batch.add_column(sa.Column("disputed_by_id", sa.String(length=36), nullable=True))
        batch.add_column(sa.Column("verification", sa.String(length=40), nullable=False, server_default="unverified"))

    op.create_index("ix_memories_role", "memories", ["role"])
    op.create_index("ix_memories_scope", "memories", ["scope"])
    op.create_index("ix_memories_source_kind", "memories", ["source_kind"])
    op.create_index("ix_memories_task_id", "memories", ["task_id"])
    op.create_index("ix_memories_superseded_by_id", "memories", ["superseded_by_id"])
    op.create_index("ix_memories_disputed_by_id", "memories", ["disputed_by_id"])
    op.create_index("ix_memories_verification", "memories", ["verification"])
    op.create_index("memory_owner_role_scope", "memories", ["owner_id", "role", "scope"])
    op.create_index("memory_owner_task", "memories", ["owner_id", "task_id"])

    # Backfill lifecycle_state through LEGACY_LIFECYCLE_MAP so legacy
    # "established" / "latent" rows become the new vocabulary.
    for legacy, mapped in LEGACY_LIFECYCLE_MAP.items():
        bind.execute(
            sa.text("UPDATE memories SET lifecycle_state = :mapped WHERE lifecycle_state = :legacy"),
            {"mapped": mapped, "legacy": legacy},
        )

    op.create_table(
        "memory_evidences",
        sa.Column("id", sa.String(length=36), primary_key=True),
        sa.Column("owner_id", sa.String(length=36), nullable=False),
        sa.Column("data", sa.JSON, nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("kind", sa.String(length=20), nullable=False, server_default="observation"),
        sa.Column("ref", sa.String(length=500), nullable=False, server_default=""),
        sa.Column("summary", sa.Text(), nullable=False, server_default=""),
        sa.Column("memory_id", sa.String(length=36), nullable=False),
        sa.Column("captured_at", sa.DateTime(timezone=True), nullable=False),
    )
    op.create_index("ix_memory_evidences_memory_id", "memory_evidences", ["memory_id"])
    op.create_index("ix_memory_evidences_kind", "memory_evidences", ["kind"])
    op.create_index("ix_memory_evidences_captured_at", "memory_evidences", ["captured_at"])
    op.create_index("ix_memory_evidences_owner_id", "memory_evidences", ["owner_id"])
    op.create_index("memory_evidence_owner_memory", "memory_evidences", ["owner_id", "memory_id"])
    op.create_index("memory_evidence_owner_kind", "memory_evidences", ["owner_id", "kind"])

    op.create_table(
        "memory_conflicts",
        sa.Column("id", sa.String(length=36), primary_key=True),
        sa.Column("owner_id", sa.String(length=36), nullable=False),
        sa.Column("data", sa.JSON, nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("memory_a_id", sa.String(length=36), nullable=False),
        sa.Column("memory_b_id", sa.String(length=36), nullable=False),
        sa.Column("reason", sa.Text(), nullable=False, server_default=""),
        sa.Column("resolved", sa.Boolean(), nullable=False, server_default=sa.text("false")),
        sa.Column("resolution", sa.String(length=40), nullable=False, server_default="unresolved"),
        sa.Column("resolved_at", sa.DateTime(timezone=True), nullable=True),
    )
    op.create_index("ix_memory_conflicts_memory_a_id", "memory_conflicts", ["memory_a_id"])
    op.create_index("ix_memory_conflicts_memory_b_id", "memory_conflicts", ["memory_b_id"])
    op.create_index("ix_memory_conflicts_resolution", "memory_conflicts", ["resolution"])
    op.create_index("ix_memory_conflicts_owner_id", "memory_conflicts", ["owner_id"])
    op.create_index("memory_conflict_owner_resolved", "memory_conflicts", ["owner_id", "resolved", "created_at"])
    op.create_unique_constraint(
        "uq_memory_conflict_owner_pair", "memory_conflicts", ["owner_id", "memory_a_id", "memory_b_id"],
    )


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
