"""Unified associative memory traces and graph indexes."""

import json
import re
from datetime import UTC, datetime
from uuid import uuid4

import sqlalchemy as sa
from sqlalchemy.dialects.postgresql import JSONB
from alembic import op

revision = "0009"
down_revision = "0008"


def upgrade():
    bind = op.get_bind()
    existing_columns = {column["name"] for column in sa.inspect(bind).get_columns("memories")}
    columns = (
        sa.Column("lifecycle_state", sa.String(20), nullable=False, server_default="established"),
        sa.Column("strength", sa.Float(), nullable=False, server_default="0.6"),
        sa.Column("confidence", sa.Float(), nullable=False, server_default="0.7"),
        sa.Column("salience", sa.Float(), nullable=False, server_default="0.5"),
        sa.Column("activation_count", sa.Integer(), nullable=False, server_default="0"),
        sa.Column("last_activated_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("last_reinforced_at", sa.DateTime(timezone=True), nullable=True),
        # Existing rows are backfilled below and ORM writes always provide a value.
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=True),
    )
    for column in columns:
        if column.name not in existing_columns:
            op.add_column("memories", column)
    existing_indexes = {index["name"] for index in sa.inspect(bind).get_indexes("memories")}
    if "ix_memories_lifecycle_state" not in existing_indexes:
        op.create_index("ix_memories_lifecycle_state", "memories", ["lifecycle_state"])
    if "memory_owner_state_created" not in existing_indexes:
        op.create_index("memory_owner_state_created", "memories", ["owner_id", "lifecycle_state", "created_at"])
    from mix_agent.db.models import MemoryActionEvent, MemoryAssociation, MemoryFeature, MemoryProcessingJob
    for table in (MemoryAssociation.__table__, MemoryFeature.__table__, MemoryProcessingJob.__table__, MemoryActionEvent.__table__):
        table.create(bind, checkfirst=True)
    rows = list(bind.execute(sa.text("SELECT id, owner_id, data, created_at FROM memories")).mappings())
    for row in rows:
        data = row["data"] if isinstance(row["data"], dict) else json.loads(row["data"] or "{}")
        importance = max(1, min(5, int(data.get("importance", 2))))
        strength = min(1.0, 0.35 + importance * 0.1 + (0.15 if data.get("pinned") else 0))
        salience = min(1.0, 0.25 + importance * 0.12 + (0.15 if data.get("pinned") else 0))
        metadata = dict(data.get("metadata") or {})
        if data.get("category"):
            metadata["legacy_category"] = data["category"]
        data["metadata"] = metadata
        state = "deleted" if data.get("deleted") else "established"
        update_sql = "UPDATE memories SET data=:data, lifecycle_state=:state, strength=:strength, confidence=:confidence, salience=:salience, updated_at=:updated WHERE id=:id"
        bind.execute(sa.text(update_sql).bindparams(sa.bindparam("data", type_=JSONB)), {
            "id": row["id"], "data": data, "state": state, "strength": strength,
            "confidence": 0.9 if data.get("source_run") == "explicit-user-request" else 0.7,
            "salience": salience, "updated": row["created_at"],
        })
        text = re.sub(r"\s+", " ", f"{data.get('content', '')} {data.get('gist', '')}".casefold())
        latin = re.findall(r"[\w-]{2,}", text, flags=re.UNICODE)
        japanese = re.findall(r"[\u3040-\u30ff\u3400-\u9fff]{2,}", text)
        features = set(latin + [part[index:index + 2] for part in japanese for index in range(len(part) - 1)])
        for value in list(features)[:1000]:
            bind.execute(MemoryFeature.__table__.insert().values(id=str(uuid4()), owner_id=row["owner_id"], memory_id=row["id"], kind="term", value=value[:160], weight=1.0, data={}, created_at=datetime.now(UTC)))


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
