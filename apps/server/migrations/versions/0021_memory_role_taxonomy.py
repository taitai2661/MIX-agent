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

    # 0001 creates every current model table on a fresh database, so each step
    # below is guarded the same way 0010 guards its table: only apply what is
    # still missing, keeping the upgrade re-runnable end to end.
    inspector = sa.inspect(bind)
    existing_columns = {column["name"] for column in inspector.get_columns("memories")}
    missing_columns = [
        column
        for column in (
            sa.Column("role", sa.String(length=20), nullable=False, server_default="fact"),
            sa.Column("scope", sa.String(length=20), nullable=False, server_default="user"),
            sa.Column("source_kind", sa.String(length=20), nullable=False, server_default="agent"),
            sa.Column("task_id", sa.String(length=36), nullable=True),
            sa.Column("superseded_by_id", sa.String(length=36), nullable=True),
            sa.Column("disputed_by_id", sa.String(length=36), nullable=True),
            sa.Column("verification", sa.String(length=40), nullable=False, server_default="unverified"),
        )
        if column.name not in existing_columns
    ]
    if missing_columns:
        with op.batch_alter_table("memories") as batch:
            for column in missing_columns:
                batch.add_column(column)

    existing_indexes = {index["name"] for index in inspector.get_indexes("memories")}
    for name, columns in (
        ("ix_memories_role", ["role"]),
        ("ix_memories_scope", ["scope"]),
        ("ix_memories_source_kind", ["source_kind"]),
        ("ix_memories_task_id", ["task_id"]),
        ("ix_memories_superseded_by_id", ["superseded_by_id"]),
        ("ix_memories_disputed_by_id", ["disputed_by_id"]),
        ("ix_memories_verification", ["verification"]),
        ("memory_owner_role_scope", ["owner_id", "role", "scope"]),
        ("memory_owner_task", ["owner_id", "task_id"]),
    ):
        if name not in existing_indexes:
            op.create_index(name, "memories", columns)

    # Backfill lifecycle_state through LEGACY_LIFECYCLE_MAP so legacy
    # "established" / "latent" rows become the new vocabulary.
    for legacy, mapped in LEGACY_LIFECYCLE_MAP.items():
        bind.execute(
            sa.text("UPDATE memories SET lifecycle_state = :mapped WHERE lifecycle_state = :legacy"),
            {"mapped": mapped, "legacy": legacy},
        )

    if "memory_evidences" not in sa.inspect(bind).get_table_names():
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
    evidence_indexes = {index["name"] for index in sa.inspect(bind).get_indexes("memory_evidences")}
    for name, columns in (
        ("ix_memory_evidences_memory_id", ["memory_id"]),
        ("ix_memory_evidences_kind", ["kind"]),
        ("ix_memory_evidences_captured_at", ["captured_at"]),
        ("ix_memory_evidences_owner_id", ["owner_id"]),
        ("memory_evidence_owner_memory", ["owner_id", "memory_id"]),
        ("memory_evidence_owner_kind", ["owner_id", "kind"]),
    ):
        if name not in evidence_indexes:
            op.create_index(name, "memory_evidences", columns)

    if "memory_conflicts" not in sa.inspect(bind).get_table_names():
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
    conflict_inspector = sa.inspect(bind)
    conflict_indexes = {index["name"] for index in conflict_inspector.get_indexes("memory_conflicts")}
    for name, columns in (
        ("ix_memory_conflicts_memory_a_id", ["memory_a_id"]),
        ("ix_memory_conflicts_memory_b_id", ["memory_b_id"]),
        ("ix_memory_conflicts_resolution", ["resolution"]),
        ("ix_memory_conflicts_owner_id", ["owner_id"]),
        ("memory_conflict_owner_resolved", ["owner_id", "resolved", "created_at"]),
    ):
        if name not in conflict_indexes:
            op.create_index(name, "memory_conflicts", columns)
    unique_constraints = {item["name"] for item in conflict_inspector.get_unique_constraints("memory_conflicts")}
    if "uq_memory_conflict_owner_pair" not in unique_constraints:
        op.create_unique_constraint(
            "uq_memory_conflict_owner_pair", "memory_conflicts", ["owner_id", "memory_a_id", "memory_b_id"],
        )


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
