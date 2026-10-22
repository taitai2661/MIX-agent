"""Phase 4 (decision A = plan B): the single ``memory_links`` table of §4.

``0024`` restored spec §1's eight columns and explicitly left *associations*
owed: "associations still owe §4's single ``memory_links`` table". This
revision is that table — exactly the columns §4 publishes:

    id / owner_id / source_memory_id / target_memory_id / kind /
    weight (default 0.2) / coactivation_count (default 0) /
    created_at / updated_at

plus §4's uniqueness rule: ``(owner_id, source_memory_id, target_memory_id,
kind)`` is unique, so an upsert of the same edge updates one row instead of
adding a second one.

What this revision deliberately does *not* restore: ``memory_associations`` /
``memory_features`` / ``memory_conflicts`` / ``memory_evidences``. Decision A =
plan B keeps the four 0022 tables deleted and takes **one** table instead —
supersession and conflict stay expressed on the row (§1's ``superseded_by_id``
/ ``disputed_by_id``) and mirror into ``kind=supersedes`` / ``kind=contradicts``
here rather than owning a second copy of that state.

Re-runnable by construction, on 0024's rules:

* the table is created only when the inspector reports it missing;
* the backfill is plain ``bind.execute(...)`` with **no** ``try/except`` — a
  backfill that fails has to fail the migration and show up, not disappear
  into a silent ``pass`` (0022's mistake);
* the backfill is an ``INSERT ... ON CONFLICT DO NOTHING`` against the unique
  constraint above, so running it twice over the same rows inserts nothing
  (idempotent without hiding failures: a real error still aborts);
* every inserted row joins ``memories`` on *both* endpoints, so a pointer to a
  row that no longer exists is skipped instead of violating the foreign key.

Backfill contract (spec §4): the link table mirrors §3's columns, so existing
data is derived from them — ``superseded_by_id`` becomes ``kind=supersedes``
and ``disputed_by_id`` becomes ``kind=contradicts``, both with the row the
column points *at* as ``source`` and the row carrying the column as
``target``. Rows written before 0024 carry no pointers, so they add nothing.

The Docker edition is PostgreSQL-only (``mix_agent.config`` refuses any other
URL), so ``gen_random_uuid()`` and ``ON CONFLICT`` below are used on purpose.
"""

import sqlalchemy as sa
from alembic import op

revision = "0025"
down_revision = "0024"

# Spec §2 `relation_kinds`, inlined so the backfill can validate the kind it
# writes without importing the implementation (parity keeps these in step).
RELATION_KINDS = (
    "supports",
    "contradicts",
    "derived_from",
    "supersedes",
    "caused_by",
    "relates_to",
)
SUPERSEDES = "supersedes"
CONTRADICTS = "contradicts"

# §4: weight defaults to 0.2, coactivation_count to 0; a backfilled edge has
# never been co-activated.
MIRROR_BACKFILL = (
    (
        SUPERSEDES,
        "superseded_by_id",
        # source = the replacement the column names, target = the retired row:
        # reading the edge as "source supersedes target" states the same fact
        # the column states as "target is superseded by source".
    ),
    (
        CONTRADICTS,
        "disputed_by_id",
        # source = the challenger the column names, target = the disputed row.
    ),
)


def upgrade():
    bind = op.get_bind()
    if bind.dialect.name != "postgresql":
        raise RuntimeError("0025 expects PostgreSQL (the Docker edition is PostgreSQL-only)")

    inspector = sa.inspect(bind)
    if not inspector.has_table("memory_links"):
        op.create_table(
            "memory_links",
            sa.Column("id", sa.String(36), primary_key=True),
            sa.Column("owner_id", sa.String(36), nullable=False),
            sa.Column(
                "source_memory_id",
                sa.String(36),
                sa.ForeignKey("memories.id", ondelete="CASCADE"),
                nullable=False,
            ),
            sa.Column(
                "target_memory_id",
                sa.String(36),
                sa.ForeignKey("memories.id", ondelete="CASCADE"),
                nullable=False,
            ),
            sa.Column("kind", sa.String(20), nullable=False),
            sa.Column("weight", sa.Float(), nullable=False, server_default="0.2"),
            sa.Column("coactivation_count", sa.Integer(), nullable=False, server_default="0"),
            sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
            sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
            sa.UniqueConstraint(
                "owner_id",
                "source_memory_id",
                "target_memory_id",
                "kind",
                name="uq_memory_links_owner_source_target_kind",
            ),
        )
        op.create_index("memory_link_owner_source", "memory_links", ["owner_id", "source_memory_id"])
        op.create_index("memory_link_owner_target", "memory_links", ["owner_id", "target_memory_id"])

    # --- backfill. No try/except: a failure must be visible. ----------------
    # §4's mirror, derived from the §3 columns 0024 restored. The join on both
    # endpoints keeps the foreign key honest, the join on owner_id keeps a
    # pointer from ever becoming a cross-owner edge, and ON CONFLICT DO NOTHING
    # makes a second run insert nothing rather than fail on the uniqueness rule.
    for kind, column in MIRROR_BACKFILL:
        if kind not in RELATION_KINDS:
            raise RuntimeError(f"spec §2 does not define the relation kind {kind!r}")
        # `column` and `kind` come from this module's own tuple, never from
        # data; the SQL is built from them so a schema surprise fails loudly.
        bind.execute(
            sa.text(
                f"INSERT INTO memory_links "
                f"(id, owner_id, source_memory_id, target_memory_id, kind, "
                f" weight, coactivation_count, created_at, updated_at) "
                f"SELECT gen_random_uuid()::text, m.owner_id, m.{column}, m.id, '{kind}', "
                f" 0.2, 0, now(), now() "
                f"FROM memories m "
                f"JOIN memories peer ON peer.id = m.{column} AND peer.owner_id = m.owner_id "
                f"ON CONFLICT DO NOTHING"
            )
        )


def downgrade():
    raise RuntimeError("Destructive downgrade is not supported; restore a verified backup.")
