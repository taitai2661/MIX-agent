"""Memory recall: scoring, ranking and the search entry point.

Pure ranking lives here so the shared recall fixture can exercise it without a
database; ``search`` adds the SQL candidate read, lifecycle gate and one-hop
link expansion.
"""

from __future__ import annotations

from datetime import UTC, datetime

from sqlalchemy import select

from mix_agent.db.models import Memory
from mix_agent.memory import embeddings as mem_embeddings
from mix_agent.memory.common import (
    CANDIDATE_LIMIT,
    LEXICAL_WEIGHT_WITH_VECTOR,
    LEXICAL_WEIGHT_WITHOUT_VECTOR,
    LIFECYCLES,
    LOGGER,
    MIN_SCORE,
    RECENCY_WEIGHT_WITHOUT_VECTOR,
    RECENCY_WEIGHT_WITH_VECTOR,
    RESULT_LIMIT_DEFAULT,
    RESULT_LIMIT_MAX,
    VECTOR_WEIGHT_WITH_VECTOR,
    _content,
    _event,
    _trace_data,
    retrievable,
    scope_weight,
    terms,
)
from mix_agent.memory.lifecycle import reinforce_row
from mix_agent.memory.links import _coactivate, _expand_links


def _score(
    query_terms: set[str], row_terms: set[str], updated_at, query_vec=None, stored_vec=None
) -> tuple[float, dict]:
    overlap = len(query_terms & row_terms) / max(1, len(query_terms)) if query_terms else 0.0
    try:
        ts = updated_at
        if ts is not None and ts.tzinfo is None:
            ts = ts.replace(tzinfo=UTC)
        age_days = max(0.0, (datetime.now(UTC) - ts).total_seconds() / 86400) if ts else 365.0
    except Exception:  # noqa: BLE001 - an unreadable timestamp falls back to the default age
        age_days = 365.0
    recency = 1 / (1 + age_days / 180)
    vector = mem_embeddings.cosine01(query_vec, stored_vec)
    # Spec §5.2 with §2.2: weights are chosen per (query, candidate) pair, so a
    # row that has no embedding yet is scored exactly as it was before the
    # vector term existed — introducing embedding must not demote old rows.
    if vector is None:
        value = LEXICAL_WEIGHT_WITHOUT_VECTOR * overlap + RECENCY_WEIGHT_WITHOUT_VECTOR * recency
        return value, {
            "lexical": round(overlap, 4),
            "recency": round(recency, 4),
            "weights": "without_vector",
        }
    value = (
        LEXICAL_WEIGHT_WITH_VECTOR * overlap
        + VECTOR_WEIGHT_WITH_VECTOR * vector
        + RECENCY_WEIGHT_WITH_VECTOR * recency
    )
    return value, {
        "lexical": round(overlap, 4),
        "vector": round(vector, 4),
        "recency": round(recency, 4),
        "weights": "with_vector",
    }


def rank(
    query_terms: set[str],
    candidates,
    *,
    query_vector=None,
    limit: int = RESULT_LIMIT_DEFAULT,
    scope_factor=None,
) -> list[dict]:
    """Score, threshold, sort and cap candidates. Pure: no database, no I/O.

    ``candidates`` yields ``(row_terms, updated_at, stored_vector, payload)``.
    This is the whole of spec §5.1/§5.2 extracted from :func:`search` so the
    ranking contract in ``tests/fixtures/memory_recall_cases.json`` can be
    asserted directly rather than re-derived inside each test.

    ``scope_factor`` is an optional ``payload -> float`` damping function:
    the caller's preferred scopes are a soft signal (§1), applied to the
    score before the threshold. It is absent from the shared fixture, which
    pins the ranking formula itself.
    """
    if not query_terms:
        return []
    scored = []
    for row_terms, updated_at, stored_vec, payload in candidates:
        # §3's retrieval gate also holds here: `search` pre-filters, but rank()
        # is public, so a superseded/archived row handed in directly is still
        # not allowed to win a slot (unless it is pinned — see retrievable()).
        if not retrievable(payload):
            continue
        value, breakdown = _score(query_terms, row_terms, updated_at, query_vector, stored_vec)
        if scope_factor is not None:
            value = value * scope_factor(payload)
        # An exact lexical hit survives even a low combined score, so a genuine
        # word match is never discarded for being old.
        if value >= MIN_SCORE or query_terms & row_terms:
            scored.append((value, updated_at, payload, breakdown))
    # A wholly unrelated query returns nothing rather than the least unrelated
    # rows (avoid polluting context).
    if not scored or max(item[0] for item in scored) < MIN_SCORE:
        scored = []
    scored.sort(key=lambda item: (item[0], item[1]), reverse=True)
    return [
        {"payload": item[2], "relevance": round(item[0], 4), "breakdown": item[3]} for item in scored[:limit]
    ]


def search(
    db,
    owner,
    query="",
    scopes=None,
    *,
    settings=None,
    debug=False,
    activate=False,
    query_vector=None,
    **kwargs,
):
    limit = RESULT_LIMIT_DEFAULT
    try:
        limit = max(1, min(RESULT_LIMIT_MAX, int((settings or {}).get("result_limit", RESULT_LIMIT_DEFAULT))))
    except Exception:  # noqa: BLE001 - a malformed setting falls back to the default limit
        LOGGER.debug("invalid memory result_limit setting; using default", exc_info=True)
    query = (query or "").strip()
    query_terms = terms(query)
    rows = list(
        db.scalars(
            select(Memory)
            .where(Memory.owner_id == owner)
            .order_by(Memory.updated_at.desc())
            .limit(CANDIDATE_LIMIT)
        )
    )
    # Spec §3: retrieval only sees retrievable lifecycles (§2), plus every
    # pinned row whatever its lifecycle. The listing endpoints keep showing
    # the rest — a superseded memory is hidden from recall, not from review.
    candidates = [row for row in rows if retrievable(row)]
    expansion = {"added": [], "edges": [], "cutoff": None}
    seeds: list[dict] = []
    # §1: the caller's preferred scopes are a soft signal, not a filter.
    preferred = [str(scope).strip().lower() for scope in (scopes or []) if str(scope or "").strip()]
    if not query_terms:
        # No query: the order is recency, with the scope hint first so a
        # caller that asked for a scope sees it at the top.
        ordered = sorted(
            candidates,
            key=lambda row: (
                scope_weight(row, preferred),
                getattr(row, "updated_at", None) or row.created_at,
            ),
            reverse=True,
        )
        seeds = [
            {"id": r.id, **_trace_data(r), "relevance": 0.0, "selection_reason": "recent"}
            for r in ordered[:limit]
        ]
        result = seeds
        mode = "recent"
    else:
        ranked = rank(
            query_terms,
            (
                (
                    terms(_content(row)),
                    getattr(row, "updated_at", None) or row.created_at,
                    mem_embeddings.row_vector(row.data),
                    row,
                )
                for row in candidates
            ),
            query_vector=query_vector,
            limit=limit,
            scope_factor=(lambda row: scope_weight(row, preferred)) if preferred else None,
        )
        hits = [item["payload"] for item in ranked]
        if activate:
            # §1/§3 reactivation: a hit is exactly when a memory should be
            # reinforced. Off by default — recall feeds the context on every
            # turn, and silently rewriting recency there would reorder the
            # next search; the explicit caller asks for it.
            for row in hits:
                reinforce_row(row)
                _event(db, owner, None, "REINFORCE", row.id, None, None, "search hit")
            # §4 source (2): the same explicit search is the only place links
            # are co-created from "these came back together". Context
            # injection searches without `activate`, so it cannot.
            _coactivate(db, owner, hits)
        seeds = [
            {
                "id": item["payload"].id,
                **_trace_data(item["payload"]),
                "relevance": item["relevance"],
                "selection_reason": "語彙が一致" if item["breakdown"]["lexical"] >= 0.4 else "文脈から関連",
            }
            for item in ranked
        ]
        # §4: one hop past the seeds, degree- and count-capped. Linked rows
        # are appended *after* the hits, never mixed into their order.
        expansion = _expand_links(db, owner, seeds)
        result = [*seeds, *expansion["added"]]
        mode = "hybrid" if query_vector else "lexical"
    if debug:
        with_vector = LEXICAL_WEIGHT_WITH_VECTOR if query_vector else LEXICAL_WEIGHT_WITHOUT_VECTOR
        return {
            "memories": result,
            "debug": {
                # `stop_reason` stays a string TraceView renders verbatim:
                # "flat" while nothing was pulled in through a link, "depth"
                # once §4's one hop actually added rows. `ranking_mode` and
                # the `association_*` keys below are the additive signal.
                "considered": len(candidates),
                "kept": len(result),
                "stop_reason": "depth" if expansion["edges"] else "flat",
                "ranking_mode": mode,
                "lifecycle_excluded": len(rows) - len(candidates),
                "elapsed_ms": 0.0,
                "auto": {},
                "trace": [],
                "seed_ids": [r["id"] for r in seeds],
                # §5.3 debug key, now carrying the rows §4 added: each entry is
                # {source, target, activation} (+ `kind`), source = a seed.
                "association_expansion": expansion["edges"],
                "association_expansion_count": len(expansion["edges"]),
                "association_cutoff": expansion["cutoff"],
                "budget_exhausted": False,
                "honored_settings": [],
                "ignored_settings": [],
                "query_vector": bool(query_vector),
                "weights": {
                    "lexical": with_vector,
                    "vector": VECTOR_WEIGHT_WITH_VECTOR if query_vector else 0.0,
                    "recency": RECENCY_WEIGHT_WITH_VECTOR if query_vector else RECENCY_WEIGHT_WITHOUT_VECTOR,
                },
                "lexical": with_vector,
                "min_score": MIN_SCORE,
            },
        }
    return result


def list_traces(db, owner, query="", state=None, limit=200):
    """The flat listing, optionally narrowed to one §2 ``lifecycle_state``.

    ``state`` used to be **accepted and ignored**: a filter that does nothing
    is worse than no filter at all, because the caller cannot tell "nothing
    matched" from "I filtered wrong" — the list simply looked empty. It now
    filters exactly, and a value outside §2's ``LIFECYCLES`` is refused rather
    than silently returning an empty page.

    This is a *listing* filter, not §2's retrieval gate: someone asking for
    ``archived`` means the archived rows, so ``pinned`` gets no exemption here
    (§3 exempts pinned rows from being hidden by **retrieval**, not from being
    named by a filter).
    """
    if state is not None and state not in LIFECYCLES:
        raise ValueError(f"unknown lifecycle_state: {state!r}")
    rows = list(
        db.scalars(
            select(Memory)
            .where(Memory.owner_id == owner)
            .order_by(Memory.updated_at.desc())
            .limit(min(limit, 500))
        )
    )
    qterms = terms(query)
    if qterms:
        rows = [r for r in rows if qterms & terms(_content(r))]
    if state is not None:
        rows = [r for r in rows if (r.lifecycle_state or "") == state]
    return [{"id": r.id, **_trace_data(r)} for r in rows]
