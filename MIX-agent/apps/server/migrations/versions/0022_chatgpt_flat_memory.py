"""ChatGPT-style flat memory: content column, drop taxonomy/graph tables.

- Add ``memories.content`` TEXT + backfill from ``data->>'content'``.
- Drop taxonomy columns (role/scope/source_kind/verification/lifecycle_state
  /strength/confidence/salience/activation_count/.../task_id/...).
- Drop tables: memory_associations / memory_features / memory_conflicts /
  memory_evidences ( associations graph, hash index, disputes, citations are
  removed; memory is a flat list of user facts like ChatGPT ).
- Keep: memories / memory_revisions / memory_processing_jobs /
  memory_action_events.

Superseded in part by ``docs/memory-spec.md``: the flat *storage* of content
stays, but the spec (decided after this migration) requires the lifecycle /
conflict columns of spec §1 back, and one ``memory_links`` table instead of the
four dropped above. Those land in later migrations. The four tables themselves
stay dropped — see the NOTE in ``mix_agent.db.models``.

Follow-up migrations must stay re-runnable: every step here is guarded by an
inspector lookup because this revision is applied to databases that already
took a partial run.
"""

import sqlalchemy as sa
from alembic import op

revision = "0022"
down_revision = "0021"


def upgrade():
    bind = op.get_bind()
    inspector = sa.inspect(bind)

    # 1. content column + backfill
    mem_cols = {c["name"] for c in inspector.get_columns("memories")}
    if "content" not in mem_cols:
        with op.batch_alter_table("memories") as batch:
            batch.add_column(sa.Column("content", sa.Text(), nullable=False, server_default=""))
    # Backfill from JSONB, then gist fallback. Works on postgres + sqlite (tests use postgres).
    try:
        if bind.dialect.name == "postgresql":
            bind.execute(sa.text(
                "UPDATE memories SET content = COALESCE(NULLIF(data->>'content',''), NULLIF(data->>'gist',''), content, '') "
                "WHERE COALESCE(content,'') = ''"
            ))
            # keep minimal data mirror for old readers
            bind.execute(sa.text(
                "UPDATE memories SET data = jsonb_set(COALESCE(data,'{}'::jsonb), '{content}', to_jsonb(content), true) "
                "WHERE (data->>'content') IS DISTINCT FROM content"
            ))
        else:
            bind.execute(sa.text(
                "UPDATE memories SET content = COALESCE(json_extract(data,'$.content'), content, '') "
                "WHERE COALESCE(content,'') = ''"
            ))
    except Exception:
        pass

    # 2. drop unused tables
    for table in ("memory_associations", "memory_features", "memory_conflicts", "memory_evidences"):
        try:
            if table in inspector.get_table_names():
                op.drop_table(table)
        except Exception:
            pass

    # 3. drop taxonomy columns (guarded, re-runnable)
    inspector = sa.inspect(bind)
    mem_cols = {c["name"] for c in inspector.get_columns("memories")} if "memories" in inspector.get_table_names() else set()
    for col in (
        "lifecycle_state", "role", "scope", "source_kind", "confidence", "salience",
        "strength", "activation_count", "last_activated_at", "last_reinforced_at",
        "task_id", "superseded_by_id", "disputed_by_id", "verification",
    ):
        if col in mem_cols:
            try:
                with op.batch_alter_table("memories") as batch:
                    batch.drop_column(col)
            except Exception:
                pass

    # 4. index for ChatGPT-style recent-first listing
    try:
        existing = {i["name"] for i in sa.inspect(bind).get_indexes("memories")}
        if "memory_owner_updated" not in existing:
            op.create_index("memory_owner_updated", "memories", ["owner_id", "updated_at"])
        if bind.dialect.name == "postgresql":
            op.execute("CREATE EXTENSION IF NOT EXISTS pg_trgm")
            # trigram on flat content column (old index was on data->>'content')
            op.execute("DROP INDEX IF EXISTS memory_text_trgm")
            op.execute("CREATE INDEX IF NOT EXISTS memory_content_trgm ON memories USING gin (content gin_trgm_ops)")
    except Exception:
        pass


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
