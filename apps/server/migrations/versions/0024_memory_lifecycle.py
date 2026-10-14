"""Phase 3 (decision B): restore spec §1's eight Memory columns.

``0022`` flattened Memory to ``content`` + ``data`` and dropped the taxonomy
columns and the four graph tables. ``docs/memory-spec.md`` §1 — the contract
for both editions — needs these eight back:

    lifecycle_state / superseded_by_id / disputed_by_id /
    confidence / salience / role / scope / source_kind

What this revision deliberately does *not* restore: ``title`` / ``verification``
/ ``activation_count`` / ``task_id`` as columns (the flat UI never renders them;
``activation_count`` and ``strength`` stay in the row's ``data``, which §1.1
lets each edition choose), and the four 0021 tables — supersession and conflict
are expressed on the row itself, associations still owe §4's single
``memory_links`` table.

Re-runnable by construction, because 0022 taught us that a partial run must be
survivable:

* every column is added only when the inspector reports it missing;
* the backfill is plain ``bind.execute(...)`` with **no** ``try/except`` —
  a backfill that fails has to fail the migration and show up, not disappear
  into a silent ``pass`` (0022's mistake);
* ``SET NOT NULL`` / ``SET DEFAULT`` run *after* the backfill, so the
  constraint doubles as proof that no row was left behind.

Backfill contract (spec §2.1 / §3): every pre-existing flat row is an
unchallenged memory, so it becomes ``established`` / ``fact`` / ``user``.
``data.role`` — written by the §6 extractor since Phase 2 — is moved into the
column and wins over the default; legacy vocabulary is mapped through §2.1
(``latent`` -> ``candidate``, ``deleted`` -> ``expired``). ``source_kind`` is
derived from ``data.source`` (``user`` explicit vs ``auto`` -> §1's ``agent``).
``pinned=true`` rows get ``salience = 1.0``: §3 exempts them from decay, so
they must also be the most salient.

The Docker edition is PostgreSQL-only (``mix_agent.config`` refuses any other
URL), so the JSONB extraction below is written for PostgreSQL on purpose.
"""

import sqlalchemy as sa
from alembic import op

revision = "0024"
down_revision = "0022"

# Spec §2 vocabulary, inlined so the SQL below can validate against it without
# importing the implementation (parity keeps these and types.py in step).
ROLES = (
    "fact", "decision", "preference", "goal", "constraint", "experience",
    "failure", "solution", "observation", "hypothesis", "question", "unknown",
)
SCOPES = ("working", "task", "project", "user", "world")
SOURCES = ("user", "agent", "tool", "browser", "external", "consolidation")

# (column, type, server default) for the NOT NULL half of §1.
CONSTRAINTS = (
    ("lifecycle_state", sa.String(20), "established"),
    ("confidence", sa.Float(), "0.7"),
    ("salience", sa.Float(), "0.5"),
    ("role", sa.String(20), "fact"),
    ("scope", sa.String(20), "user"),
    ("source_kind", sa.String(20), "agent"),
)


def _quoted(values) -> str:
    return ", ".join("'{}'".format(value) for value in values)


def upgrade():
    bind = op.get_bind()
    if bind.dialect.name != "postgresql":
        raise RuntimeError("0024 expects PostgreSQL (the Docker edition is PostgreSQL-only)")

    inspector = sa.inspect(bind)
    columns = {column["name"] for column in inspector.get_columns("memories")}

    # --- 1. add what is missing (nullable first; constraints come later) -----
    additions = []
    if "lifecycle_state" not in columns:
        additions.append(sa.Column("lifecycle_state", sa.String(20), nullable=True))
    if "superseded_by_id" not in columns:
        additions.append(sa.Column("superseded_by_id", sa.String(36), nullable=True))
    if "disputed_by_id" not in columns:
        additions.append(sa.Column("disputed_by_id", sa.String(36), nullable=True))
    if "confidence" not in columns:
        additions.append(sa.Column("confidence", sa.Float(), nullable=True))
    if "salience" not in columns:
        additions.append(sa.Column("salience", sa.Float(), nullable=True))
    if "role" not in columns:
        additions.append(sa.Column("role", sa.String(20), nullable=True))
    if "scope" not in columns:
        additions.append(sa.Column("scope", sa.String(20), nullable=True))
    if "source_kind" not in columns:
        additions.append(sa.Column("source_kind", sa.String(20), nullable=True))
    if additions:
        with op.batch_alter_table("memories") as batch:
            for column in additions:
                batch.add_column(column)

    # --- 2. backfill. No try/except: a failure must be visible. -------------
    # lifecycle: a flat row was never superseded / disputed / archived.
    bind.execute(sa.text(
        "UPDATE memories SET lifecycle_state = 'established' "
        "WHERE lifecycle_state IS NULL OR lifecycle_state = ''"
    ))
    # Legacy vocabulary of §2.1 (only reachable on a database that kept a
    # pre-0022 column value), mapped rather than silently reclassified.
    bind.execute(sa.text("UPDATE memories SET lifecycle_state = 'candidate' WHERE lifecycle_state = 'latent'"))
    bind.execute(sa.text("UPDATE memories SET lifecycle_state = 'expired' WHERE lifecycle_state = 'deleted'"))

    # role: data.role (the §6 extractor, Phase 2) wins, and only a §2 role may
    # be stored — anything else falls back to §1's default.
    role_list = _quoted(ROLES)
    bind.execute(sa.text(
        "UPDATE memories SET role = data->>'role' "
        f"WHERE data->>'role' IN ({role_list}) AND role IS DISTINCT FROM data->>'role'"
    ))
    bind.execute(sa.text(
        "UPDATE memories SET role = 'fact' "
        f"WHERE role IS NULL OR role = '' OR role NOT IN ({role_list})"
    ))

    # scope: §1's default for rows that predate the column.
    bind.execute(sa.text(
        f"UPDATE memories SET scope = 'user' WHERE scope IS NULL OR scope = '' OR scope NOT IN ({_quoted(SCOPES)})"
    ))

    # source_kind: data.source is 'user' (explicit) or 'auto' (extracted);
    # 'auto' is not a §2 source, so it maps to §1's 'agent'.
    bind.execute(sa.text(
        "UPDATE memories SET source_kind = 'agent' "
        "WHERE data->>'source' = 'auto' AND source_kind IS DISTINCT FROM 'agent'"
    ))
    bind.execute(sa.text(
        "UPDATE memories SET source_kind = data->>'source' "
        f"WHERE data->>'source' IN ({_quoted(SOURCES)}) AND source_kind IS DISTINCT FROM data->>'source'"
    ))
    bind.execute(sa.text(
        "UPDATE memories SET source_kind = 'agent' "
        f"WHERE source_kind IS NULL OR source_kind = '' OR source_kind NOT IN ({_quoted(SOURCES)})"
    ))

    # confidence / salience: §1 defaults for rows that predate the columns.
    bind.execute(sa.text("UPDATE memories SET confidence = 0.7 WHERE confidence IS NULL"))
    bind.execute(sa.text("UPDATE memories SET salience = 0.5 WHERE salience IS NULL"))
    # §3: pinned rows are exempt from decay, so they start at full salience.
    # (`pinned` is a JSON boolean or a 1/0; lower() keeps the cast from ever
    # raising on a hand-edited row — a bad value must not fail the migration.)
    bind.execute(sa.text(
        "UPDATE memories SET salience = 1.0 "
        "WHERE lower(COALESCE(data->>'pinned', '')) IN ('true', '1') "
        "AND salience IS DISTINCT FROM 1.0"
    ))

    # superseded_by_id / disputed_by_id have no historical source: a flat row
    # was never superseded and never disputed.

    # --- 3. constraints last: SET NOT NULL fails if a row was missed --------
    for name, type_, default in CONSTRAINTS:
        op.alter_column(
            "memories", name,
            existing_type=type_, server_default=default, nullable=False,
        )
    for name in ("superseded_by_id", "disputed_by_id"):
        op.alter_column("memories", name, existing_type=sa.String(36), nullable=True)

    # The retrieval filter (spec §3) reads lifecycle_state for every owner.
    existing_indexes = {index["name"] for index in sa.inspect(bind).get_indexes("memories")}
    if "ix_memories_lifecycle_state" not in existing_indexes:
        op.create_index("ix_memories_lifecycle_state", "memories", ["lifecycle_state"])


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
