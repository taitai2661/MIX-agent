"""Spec §4 ``memory_links``: idempotent edges, mirrors and expansion.

Kept separate from recall and lifecycle so each module owns one direction:
``sync_links`` derives §4 from §3 columns, while this module never writes a §3
column except the detach helper used when a memory is deleted.
"""

from __future__ import annotations

from sqlalchemy import and_, inspect, or_, select

from mix_agent.db.models import Memory, MemoryLink, now
from mix_agent.memory import embeddings as mem_embeddings
from mix_agent.memory.common import (
    CANDIDATE_LIMIT,
    LINK_DEFAULT_WEIGHT,
    LINK_DEGREE_LIMIT,
    LINK_EXPANSION_LIMIT,
    LINK_NEIGHBOR_MIN_SIMILARITY,
    LINK_NEIGHBOR_TOP_K,
    LINK_SUGGEST_LIMIT,
    _content,
    _trace_data,
    clamp,
    number,
    retrievable,
    terms,
)
from mix_agent.memory.types import (
    REL_CONTRADICTS,
    REL_RELATES_TO,
    REL_SUPERSEDES,
    RELATION_KINDS,
)


def _links_available(db) -> bool:
    """Whether §4's table exists *yet*, answered without swallowing errors.

    Two callers need "not yet": the Phase 3 scratch-database test migrates a
    throw-away database only as far as 0024, and a deployment is briefly
    between revisions while ``alembic upgrade`` runs. Both must degrade to
    "no links" instead of raising on a missing table. The answer comes from
    the catalog and is cached on the session (it cannot change underneath a
    live session) — never from catching ``ProgrammingError``, which would
    also hide a genuine failure of the query itself.
    """
    cached = db.info.get("memory_links_available")
    if cached is None:
        cached = inspect(db.get_bind()).has_table("memory_links")
        db.info["memory_links_available"] = bool(cached)
    return bool(cached)


def _link_view(row: MemoryLink) -> dict:
    """§4's public edge shape, floats and ids only (no ORM objects leak)."""
    return {
        "id": row.id,
        "owner_id": row.owner_id,
        "source_memory_id": row.source_memory_id,
        "target_memory_id": row.target_memory_id,
        "kind": row.kind,
        "weight": number(row.weight, LINK_DEFAULT_WEIGHT),
        "coactivation_count": int(row.coactivation_count or 0),
        "created_at": row.created_at.isoformat() if row.created_at else None,
        "updated_at": row.updated_at.isoformat() if row.updated_at else None,
    }


def _upsert_link(db, owner, source_id, target_id, kind, *, weight=LINK_DEFAULT_WEIGHT, increment=False):
    """One row per ``(owner, source, target, kind)`` — §4's uniqueness rule.

    Returns ``(row, created)``. ``weight`` is written on creation and, for an
    existing edge, only when the caller states something other than the §4
    default: a coactivation increment must never drag a similarity-weighted
    edge back down to 0.2.
    """
    row = db.scalar(
        select(MemoryLink).where(
            MemoryLink.owner_id == owner,
            MemoryLink.source_memory_id == source_id,
            MemoryLink.target_memory_id == target_id,
            MemoryLink.kind == kind,
        )
    )
    created = row is None
    if created:
        row = MemoryLink(
            owner_id=owner,
            source_memory_id=source_id,
            target_memory_id=target_id,
            kind=kind,
            weight=clamp(weight),
            coactivation_count=1 if increment else 0,
            created_at=now(),
            updated_at=now(),
        )
        db.add(row)
    else:
        if increment:
            row.coactivation_count = int(row.coactivation_count or 0) + 1
        if weight != LINK_DEFAULT_WEIGHT:
            row.weight = clamp(weight)
        row.updated_at = now()
    db.flush()
    return row, created


def link(db, owner, source_id, target_id, kind, *, weight=LINK_DEFAULT_WEIGHT, increment=False) -> dict:
    """Spec §4: idempotent upsert of one directed edge.

    ``kind`` must be one of §2's ``relation_kinds`` — anything else raises
    ``ValueError`` first, so an unknown word never reaches the table.

    The two §3 mirror kinds are then refused **even though §2 defines them**:
    §4 writes only §4's own edges (``relates_to`` / ``supports`` /
    ``derived_from`` / ``caused_by``). ``superseded_by_id`` / ``disputed_by_id``
    are §3 columns, only a §3 transition may set them, and :func:`sync_links`
    turns them into edges — the derivation runs §3 → §4 and never back. Letting
    this function write a column instead would raise a row that carries
    ``superseded_by_id`` while its ``lifecycle_state`` stays ``established``,
    i.e. one still in §2's ``retrievable_lifecycles`` although §3's diagram has
    ``supersede`` / ``dispute`` set the state *and* the pointer together.
    """
    if kind not in RELATION_KINDS:
        raise ValueError(f"unknown relation kind: {kind!r}")
    if kind in (REL_SUPERSEDES, REL_CONTRADICTS):
        raise ValueError(
            f"{kind!r} is the mirror of §3's superseded_by_id / disputed_by_id: only a §3"
            " transition writes those columns and sync_links derives the edge from them"
            " (spec §4); link() makes §4's own edges only"
        )
    source = db.get(Memory, str(source_id)) if source_id else None
    target = db.get(Memory, str(target_id)) if target_id else None
    if source is None or source.owner_id != owner:
        raise ValueError("source memory not found")
    if target is None or target.owner_id != owner:
        raise ValueError("target memory not found")
    if source.id == target.id:
        raise ValueError("a memory cannot link to itself")
    row, _created = _upsert_link(db, owner, source.id, target.id, kind, weight=weight, increment=increment)
    return _link_view(row)


def sync_links(db, row) -> None:
    """§4's mirror: make the edges around ``row`` agree with its §3 columns.

    Called by every transition that moves ``superseded_by_id`` /
    ``disputed_by_id`` — supersede, dispute, resolve, revision restore,
    delete — inside the same transaction, so no path can leave a column
    naming an edge that is not there or an edge claiming a pointer the row no
    longer holds. Inactive rows (missing table, pre-0025 database) are a
    no-op, not an error.
    """
    if row is None or not _links_available(db):
        return
    _sync_kind(db, row, REL_SUPERSEDES, getattr(row, "superseded_by_id", None))
    _sync_kind(db, row, REL_CONTRADICTS, getattr(row, "disputed_by_id", None))


def _sync_kind(db, row, kind, source_id):
    """Reconcile one §4 edge set with one §3 column (see :func:`sync_links`)."""
    existing = list(
        db.scalars(
            select(MemoryLink).where(
                MemoryLink.owner_id == row.owner_id,
                MemoryLink.target_memory_id == row.id,
                MemoryLink.kind == kind,
            )
        )
    )
    keeper = None
    for edge in existing:
        if keeper is None and edge.source_memory_id == source_id:
            keeper = edge  # the edge the column names: keep it as it is
            continue
        db.delete(edge)  # a stale mirror (or an edge whose pointer moved)
    if source_id and keeper is None:
        pointed = db.get(Memory, str(source_id))
        # A pointer whose row is gone gets no edge — the foreign key refuses
        # one — and `sync_links` leaves the column for its caller to judge.
        if pointed is not None and pointed.owner_id == row.owner_id:
            db.add(
                MemoryLink(
                    owner_id=row.owner_id,
                    source_memory_id=pointed.id,
                    target_memory_id=row.id,
                    kind=kind,
                    weight=LINK_DEFAULT_WEIGHT,
                    coactivation_count=0,
                    created_at=now(),
                    updated_at=now(),
                )
            )
    db.flush()


def associations_for(db, owner, memory_id, limit=20):
    """Spec §4: the edges touching ``memory_id``, in both directions.

    Returned newest-heaviest first and capped by ``limit``; §4's endpoint is
    an inspection view, not a retrieval path, so the lifecycle gate does not
    apply here — a superseded row's edges are exactly what a reviewer wants
    to see.
    """
    if not memory_id or not _links_available(db):
        return []
    rows = list(
        db.scalars(
            select(MemoryLink)
            .where(
                MemoryLink.owner_id == owner,
                or_(MemoryLink.source_memory_id == memory_id, MemoryLink.target_memory_id == memory_id),
            )
            .order_by(
                MemoryLink.weight.desc(), MemoryLink.coactivation_count.desc(), MemoryLink.updated_at.desc()
            )
            .limit(max(1, int(limit or 20)))
        )
    )
    return [_link_view(row) for row in rows]


def link_counts(db, owner, ids) -> dict[str, int]:
    """How many edges touch each id, in one round trip (for §5.3 rendering)."""
    wanted = [str(item) for item in (ids or []) if item]
    if not wanted or not _links_available(db):
        return {}
    counts: dict[str, int] = {}
    for source_id, target_id in db.execute(
        select(MemoryLink.source_memory_id, MemoryLink.target_memory_id).where(
            MemoryLink.owner_id == owner,
            or_(MemoryLink.source_memory_id.in_(wanted), MemoryLink.target_memory_id.in_(wanted)),
        )
    ):
        counts[source_id] = counts.get(source_id, 0) + 1
        counts[target_id] = counts.get(target_id, 0) + 1
    return counts


def _coactivate(db, owner, rows) -> int:
    """§4 source (2): rows recalled together by one *explicit* search.

    Pairwise over the hit list (rank order fixes the direction), ``kind
    relates_to``, +1 per repeat. Gated on ``activate=True`` so context
    injection — which searches every turn without activating — cannot fabric
    links out of "these two were mentioned today"; that is the same reason
    §3 keeps injection from reinforcing.
    """
    ids = [row.id for row in rows]
    if len(ids) < 2:
        return 0
    created = 0
    for index, source_id in enumerate(ids):
        for target_id in ids[index + 1 :]:
            # A coactivation is symmetric, so the pair is *one* row however the
            # two came back this time: §5.2 breaks a tie on `updated_at`, which
            # the reinforcing above just moved, so the order can flip between
            # searches. An edge already stored the other way round is the same
            # fact — increment it instead of adding its mirror image.
            existing = db.scalar(
                select(MemoryLink).where(
                    MemoryLink.owner_id == owner,
                    MemoryLink.kind == REL_RELATES_TO,
                    or_(
                        and_(
                            MemoryLink.source_memory_id == source_id, MemoryLink.target_memory_id == target_id
                        ),
                        and_(
                            MemoryLink.source_memory_id == target_id, MemoryLink.target_memory_id == source_id
                        ),
                    ),
                )
            )
            if existing is not None:
                existing.coactivation_count = int(existing.coactivation_count or 0) + 1
                existing.updated_at = now()
                db.flush()
                continue
            _row, is_new = _upsert_link(db, owner, source_id, target_id, REL_RELATES_TO, increment=True)
            created += 1 if is_new else 0
    return created


def _expand_links(
    db, owner, seeds, *, degree_limit=LINK_DEGREE_LIMIT, total_limit=LINK_EXPANSION_LIMIT
) -> dict:
    """§4: exactly one hop out of the seed rows — never two.

    The seeds are the rows the query actually matched; a linked row is
    appended *after* them with ``selection_reason="リンクから"`` and a
    relevance the seed's relevance was scaled down by the edge weight, so it
    can neither outrank a real hit nor be mistaken for one. Both §4 caps are
    honoured here: ``degree_limit`` edges per seed, ``total_limit`` rows per
    search, whichever comes first (``cutoff`` says which one did).

    ``retrievable()`` is applied again on the way in: a link into a
    superseded or archived memory does not resurrect it, while a pinned one
    stays reachable (§3 exempts pinned rows from every automatic retirement).
    """
    empty = {"added": [], "edges": [], "cutoff": None}
    if not seeds or not _links_available(db):
        return empty
    seed_ids = [seed["id"] for seed in seeds]
    seed_set = set(seed_ids)
    relevance_of = {seed["id"]: number(seed.get("relevance"), 0.0) for seed in seeds}
    links = list(
        db.scalars(
            select(MemoryLink)
            .where(
                MemoryLink.owner_id == owner,
                or_(MemoryLink.source_memory_id.in_(seed_ids), MemoryLink.target_memory_id.in_(seed_ids)),
            )
            .order_by(
                MemoryLink.weight.desc(), MemoryLink.coactivation_count.desc(), MemoryLink.updated_at.desc()
            )
        )
    )
    reached: dict[str, tuple[MemoryLink, str]] = {}
    degree: dict[str, int] = {}
    degree_truncated = False
    for edge in links:
        seed_id = edge.source_memory_id if edge.source_memory_id in seed_set else edge.target_memory_id
        other_id = edge.target_memory_id if seed_id == edge.source_memory_id else edge.source_memory_id
        if other_id in seed_set:
            continue  # seed ↔ seed: both rows are already in the result
        if degree.get(seed_id, 0) >= degree_limit:
            degree_truncated = True  # §4's degree cap stops this seed here
            continue
        degree[seed_id] = degree.get(seed_id, 0) + 1
        if other_id not in reached:
            reached[other_id] = (edge, seed_id)
    added: list[dict] = []
    edges: list[dict] = []
    cutoff = None
    for other_id, (edge, seed_id) in reached.items():
        if len(added) >= total_limit:
            cutoff = "expansion_limit"
            break
        row = db.get(Memory, other_id)
        if row is None or row.owner_id != owner or not retrievable(row):
            continue
        weight = number(edge.weight, LINK_DEFAULT_WEIGHT)
        added.append(
            {
                "id": row.id,
                **_trace_data(row),
                "relevance": round(relevance_of.get(seed_id, 0.0) * weight, 4),
                "selection_reason": "リンクから",
            }
        )
        edges.append({"source": seed_id, "target": row.id, "activation": weight, "kind": edge.kind})
    if cutoff is None and degree_truncated:
        cutoff = "degree_limit"
    return {"added": added, "edges": edges, "cutoff": cutoff}


def suggest_links(db, owner, memory_ids, *, limit=LINK_SUGGEST_LIMIT) -> int:
    """§4 source (1): the links an extraction *would* have proposed.

    §6's prompt is a verbatim cross-edition contract — parity byte-compares
    it against the Desktop copy and ``test_memory_extraction_quality``
    greps the spec for it — so asking the model for a ``links`` key would
    mean changing §6 and the other edition in lockstep, not adding a field.
    The proposal is therefore derived after ``jobs._apply`` from what §5
    already agrees on: the new row's terms against the owner's store, ranked
    by the same :func:`rank` §4's retrieval uses, capped at ``limit`` per
    memory and restricted to rows §3 still lets retrieval see.
    """
    ids = [str(item) for item in (memory_ids or []) if item]
    if not ids or not _links_available(db):
        return 0
    # Local import: recall imports this module for one-hop expansion, so a
    # module-level import here would be circular.
    from mix_agent.memory.recall import rank

    pool = list(
        db.scalars(
            select(Memory).where(Memory.owner_id == owner).order_by(Memory.updated_at.desc()).limit(200)
        )
    )
    created = 0
    for memory_id in ids:
        row = db.get(Memory, memory_id)
        if row is None or row.owner_id != owner:
            continue
        query_terms = terms(_content(row))
        if not query_terms:
            continue
        matches = [other for other in pool if other.id != row.id and query_terms & terms(_content(other))]
        for item in rank(
            query_terms,
            (
                (
                    terms(_content(other)),
                    other.updated_at or other.created_at,
                    mem_embeddings.row_vector(other.data),
                    other,
                )
                for other in matches
            ),
            limit=limit,
        ):
            _edge, is_new = _upsert_link(db, owner, row.id, item["payload"].id, REL_RELATES_TO)
            created += 1 if is_new else 0
    return created


def embedding_neighbor_links(
    db, owner, memory_ids, *, top_k=LINK_NEIGHBOR_TOP_K, minimum=LINK_NEIGHBOR_MIN_SIMILARITY
) -> int:
    """§4 source (3): §5 embedding neighbours — **既定オフ**.

    :func:`jobs._apply` only calls this when the ``memory_link_embedding_
    neighbors`` setting is true: links made here grow with the square of the
    store, and §4 marks the source off by default for exactly that reason.
    Rows without a vector, and rows §3 would not retrieve, never become
    neighbours.
    """
    ids = [str(item) for item in (memory_ids or []) if item]
    if not ids or not _links_available(db):
        return 0
    pool = [
        row
        for row in db.scalars(
            select(Memory)
            .where(Memory.owner_id == owner)
            .order_by(Memory.updated_at.desc())
            .limit(CANDIDATE_LIMIT)
        )
        if row.id not in ids and retrievable(row)
    ]
    created = 0
    for memory_id in ids:
        row = db.get(Memory, memory_id)
        if row is None or row.owner_id != owner:
            continue
        vector = mem_embeddings.row_vector(row.data)
        if vector is None:
            continue
        scored = []
        for other in pool:
            similarity = mem_embeddings.cosine01(vector, mem_embeddings.row_vector(other.data))
            if similarity is None or similarity < minimum:
                continue
            scored.append((similarity, other))
        scored.sort(key=lambda item: item[0], reverse=True)
        for similarity, other in scored[:top_k]:
            _edge, is_new = _upsert_link(
                db,
                owner,
                row.id,
                other.id,
                REL_RELATES_TO,
                weight=max(LINK_DEFAULT_WEIGHT, round(similarity, 2)),
            )
            created += 1 if is_new else 0
    return created


def _detach_pointers(db, owner, memory_id) -> None:
    """Clear every §3 pointer that named a row that is being deleted.

    §4 forbids the half-updated state: the edges go with the row (the foreign
    key cascades), so a column left pointing at a memory that no longer
    exists must go too — otherwise the mirror would claim a link for a row
    that is not there.
    """
    for name, column in (
        ("superseded_by_id", Memory.superseded_by_id),
        ("disputed_by_id", Memory.disputed_by_id),
    ):
        for row in list(db.scalars(select(Memory).where(Memory.owner_id == owner, column == memory_id))):
            setattr(row, name, None)
            db.flush()
            sync_links(db, row)
