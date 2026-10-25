"""Memory Runtime: flat recall + a plan applier that implements spec §3.

- ``recall``: text-match + recency over flat Memory rows, filtered by §3's
  retrieval gate (retrievable lifecycles + everything pinned).
- ``evaluate``: dedupe into ``reinforce``, otherwise ``create``.
- ``apply_plan``: ``create`` / ``reinforce`` / ``update`` / ``supersede`` /
  ``dispute`` / ``no_op``. ``supersede`` is *not* a delete (§3): the
  replacement row is created, the old row survives as ``superseded`` with
  ``superseded_by_id`` pointing at it, and a revision of the old row is
  written first. ``dispute`` marks the row and keeps it.
- ``resolve_conflict``: §3's settlement — one row back to ``active``, the
  other retired to ``superseded``/``expired``; both rows stay.
- ``group_for_context``: single ``all`` bucket (+ legacy bucket keys as
  empty lists so old readers don't crash).
"""

from __future__ import annotations

import logging

from sqlalchemy import func, select

from mix_agent.db.models import Memory, MemoryActionEvent, MemoryRevision, now
from mix_agent.memory import service as legacy_service
from mix_agent.memory import views as mem_views
from mix_agent.memory.service import (
    ITEM_CHAR_LIMIT,
    LEXICAL_WEIGHT_WITH_VECTOR,
    LEXICAL_WEIGHT_WITHOUT_VECTOR,
    RECENCY_WEIGHT_WITHOUT_VECTOR,
    RESULT_LIMIT_DEFAULT,
    RESULT_LIMIT_MAX,
)
from mix_agent.memory.types import LIFECYCLE_DISPUTED, LIFECYCLE_SUPERSEDED

LOGGER = logging.getLogger(__name__)

RECALL_DEFAULTS: dict[str, float] = {
    "result_limit": legacy_service.RESULT_LIMIT_DEFAULT,
    "lexical_weight": legacy_service.LEXICAL_WEIGHT_WITHOUT_VECTOR,
    "recency_weight": legacy_service.RECENCY_WEIGHT_WITHOUT_VECTOR,
}


def _query_text(task) -> str:
    if isinstance(task, str):
        return task
    if isinstance(task, dict):
        for key in ("text", "goal", "question", "intent"):
            value = task.get(key)
            if isinstance(value, str) and value.strip():
                return value
        return " ".join(str(v) for v in task.values() if isinstance(v, str))[:2000]
    return ""


def recall(
    db,
    owner: str,
    task,
    *,
    scopes=None,
    roles=None,
    include_lifecycles=None,
    limit=None,
    settings=None,
    debug=False,
    query_vector=None,
    **kwargs,
):
    query = _query_text(task)
    n = max(
        1,
        min(
            RESULT_LIMIT_MAX,
            int(
                limit or (settings or {}).get("result_limit", RESULT_LIMIT_DEFAULT)
                if isinstance(settings, dict)
                else RESULT_LIMIT_DEFAULT
            ),
        ),
    )
    found = legacy_service.search(
        db, owner, query, scopes=scopes, settings={"result_limit": n}, debug=debug, query_vector=query_vector
    )
    search_debug: dict = {}
    if debug:
        # `search(debug=True)` wraps its rows; unwrap so both branches below
        # render from the same list.
        search_debug = found.get("debug") or {}
        found = found.get("memories") or []
    views = []
    # §4/§5.3: one batched count so a rendered hit can say how many edges it
    # has (0 when none) without a query per row.
    counts = legacy_service.link_counts(db, owner, [item["id"] for item in found])
    for item in found:
        row = db.get(Memory, item["id"])
        if not row:
            continue
        views.append(
            mem_views.render(
                row,
                score=item.get("relevance"),
                selection_reason=item.get("selection_reason"),
                link_count=counts.get(item["id"], 0),
            )
        )
    if debug:
        weights = search_debug.get("weights") or {
            "lexical": LEXICAL_WEIGHT_WITH_VECTOR if query_vector else LEXICAL_WEIGHT_WITHOUT_VECTOR,
            "vector": 0.0,
            "recency": RECENCY_WEIGHT_WITHOUT_VECTOR,
        }
        return {
            "memories": views,
            "debug": {
                "considered": search_debug.get("considered", len(views)),
                "kept": len(views),
                "weights": weights,
                "lexical": search_debug.get(
                    "lexical", LEXICAL_WEIGHT_WITH_VECTOR if query_vector else LEXICAL_WEIGHT_WITHOUT_VECTOR
                ),
                "query_vector": bool(query_vector),
                "task_keywords": sorted(legacy_service.terms(query))[:20],
                # Honest values (spec §1): what the results actually *are*,
                # read back from the rows instead of a hard-coded `["active"]`.
                "scopes": sorted({item.get("scope") for item in found if item.get("scope")}),
                "roles": sorted({item.get("role") for item in found if item.get("role")}),
                "lifecycles": sorted(
                    {item.get("lifecycle_state") for item in found if item.get("lifecycle_state")}
                ),
                "lifecycle_excluded": search_debug.get("lifecycle_excluded", 0),
                "elapsed_ms": search_debug.get("elapsed_ms", 0.0),
                "budget_exhausted": False,
                # §4/§5.3: the one hop search() followed, forwarded unchanged.
                # TraceView reads `association_expansion` entries as
                # {source, target, activation}; `stop_reason`,
                # `association_expansion_count` and `association_cutoff` add how
                # many rows came in and which §4 cap stopped the walk.
                "stop_reason": search_debug.get("stop_reason"),
                # Forwarded, not invented: `search(debug=True)` decided it, and
                # the probe path already showed it. Without this line the two
                # debug surfaces of one edition disagreed about how the rows
                # were ranked — the probe said, the injection trace did not.
                "ranking_mode": search_debug.get("ranking_mode"),
                "association_expansion": search_debug.get("association_expansion") or [],
                "association_expansion_count": search_debug.get("association_expansion_count", 0),
                "association_cutoff": search_debug.get("association_cutoff"),
            },
        }
    return views


def evaluate(
    db,
    owner: str,
    *,
    task_id=None,
    observation=None,
    tool_call=None,
    plan_step=None,
    explicit_user=False,
    **kwargs,
) -> dict:
    observation = observation or {}
    summary = (observation.get("summary") or observation.get("content") or "").strip()
    if not summary:
        return {"plan": [], "skipped": True, "reason": "empty summary"}
    content = legacy_service.redact_sensitive((observation.get("content") or summary)[:4000])
    summary = legacy_service.redact_sensitive(summary[:600])
    if legacy_service.SENSITIVE.search(summary) or legacy_service.SENSITIVE.search(content):
        return {"plan": [], "skipped": True, "reason": "contains credentials"}
    if observation.get("temporary"):
        return {"plan": [], "skipped": True, "reason": "marked temporary"}
    if len(content.strip()) < 2:
        return {"plan": [], "skipped": True, "reason": "too short"}
    normalized = legacy_service._normalize(content)
    for row in db.scalars(
        select(Memory).where(Memory.owner_id == owner).order_by(Memory.updated_at.desc()).limit(200)
    ):
        existing = legacy_service._content(row)
        if legacy_service._normalize(existing) == normalized:
            return {
                "plan": [{"action": "reinforce", "memory_id": row.id, "reason": "duplicate"}],
                "skipped": False,
                "reason": "duplicate",
            }
    candidate = {"content": content, "gist": summary, "explicit_user": explicit_user}
    # Spec §6's extractor labels each item; keep the label (validated) so the
    # created row carries `data.role` instead of pretending everything is fact.
    role = legacy_service.role_value(observation.get("role"))
    if role:
        candidate["role"] = role
    return {"plan": [{"action": "create", "candidate": candidate}], "skipped": False, "reason": "new"}


def apply_plan(db, owner: str, plan: list[dict], *, run_id: str | None = None) -> list[dict]:
    """Persist a spec §3 plan, logging each step's ``MemoryActionEvent`` once.

    ``logged`` is what keeps that promise: the ``create`` / ``reinforce`` /
    ``update`` / ``supersede`` / ``dispute`` branches write their own event
    (with the memory id the generic fallback cannot know), so the trailing
    emitter below must not count the same step again.
    """
    results: list[dict] = []
    for step in plan or []:
        action = step.get("action")
        reason = str(step.get("reason", ""))
        logged = False
        if action == "create":
            candidate = step.get("candidate") or {}
            content = str(candidate.get("content") or candidate.get("gist") or "").strip()
            if not content:
                continue
            # §3 initial lifecycle: a plan built from a human's explicit
            # remember is stored `established` (like the synchronous path);
            # an automatic one stays `candidate` until something confirms it.
            source_run = run_id or ("explicit-user-request" if candidate.get("explicit_user") else None)
            created = legacy_service.change(
                db, owner, content, source_run=source_run, pinned=False, role=candidate.get("role")
            )
            row = db.get(Memory, created["id"])
            results.append({"action": "create", "id": row.id, "memory": mem_views.render(row)})
            logged = True  # service.change already logged CREATE (or REINFORCE on dedupe)
        elif action == "reinforce":
            row = db.get(Memory, step.get("memory_id"))
            if row and row.owner_id == owner:
                # §1/§3: an activation moves recency *and* the strength fields
                # (a bare `updated_at` bump forgot half of what reinforce is).
                legacy_service.reinforce_row(row, reason="plan reinforce")
                results.append({"action": "reinforce", "id": row.id})
                if run_id:
                    _emit_event(db, owner, run_id, "REINFORCE", row.id, None, None, reason)
                logged = True
        elif action == "no_op":
            continue
        elif action == "update":
            # compat: in-place content update (§7.2 `update`), not a transition.
            target = step.get("memory_id")
            candidate = step.get("candidate") or {}
            content = str(candidate.get("content") or "").strip()
            if target and content:
                try:
                    legacy_service.change(db, owner, content, memory_id=target, source_run=run_id)
                except ValueError:
                    continue
                results.append({"action": "update", "id": target})
                if run_id:
                    _emit_event(db, owner, run_id, "UPDATE", target, None, None, reason)
                logged = True
        elif action == "supersede":
            # §3: retire `memory_id` in favour of a replacement row. The step
            # hands over either the replacement id (`new_id`) or its content.
            target = str(step.get("memory_id") or "")
            candidate = step.get("candidate") or {}
            content = str(candidate.get("content") or candidate.get("gist") or "").strip()
            new_id = str(step.get("new_id") or "")
            if target and not new_id and content:
                created = legacy_service.change(db, owner, content, source_run=run_id)
                new_id = str(created.get("id") or "")
            if not target or not new_id or new_id == target:
                # Duplicate-guard: `change` deduped the replacement back onto
                # the very row being replaced — there is nothing to supersede.
                continue
            try:
                supersede(db, owner, old_id=target, new_id=new_id, reason=reason)
            except ValueError:
                continue
            results.append({"action": "supersede", "id": target, "superseded_by_id": new_id})
            logged = True
        elif action == "dispute":
            # §3: mark the row the plan names as disputed against the other
            # side, which is given directly (`conflicting_id`) or still has to
            # be stored (`candidate` content).
            target = str(step.get("memory_id") or "")
            candidate = step.get("candidate") or {}
            content = str(candidate.get("content") or candidate.get("gist") or "").strip()
            conflicting_id = str(step.get("conflicting_id") or "")
            if target and not conflicting_id and content:
                created = legacy_service.change(db, owner, content, source_run=run_id)
                conflicting_id = str(created.get("id") or "")
            if not target or not conflicting_id or target == conflicting_id:
                continue
            try:
                marked = dispute(db, owner, memory_id=target, conflicting_id=conflicting_id, reason=reason)
            except ValueError:
                continue
            results.append({"action": "dispute", **marked})
            logged = True
        else:
            LOGGER.warning("Unknown memory plan action: %s", action)
        if run_id and action and not logged:
            _emit_event(db, owner, run_id, str(action).upper(), None, None, None, reason)
    return results


def supersede(db, owner: str, *, old_id: str, new_id: str | None = None, reason: str = "") -> dict:
    """Spec §3: retire ``old_id`` in favour of ``new_id``. Never a delete.

    The old row survives as ``superseded`` with ``superseded_by_id`` pointing
    at the replacement, after a revision of its pre-transition state (§1: no
    change may lose history). A supersede needs the replacement: ``new_id=None``
    is refused instead of quietly degrading into the hard delete this function
    used to be — forgetting outright is ``service.change(..., delete=True)``,
    which keeps its own revision (see spec §3 "forget ≠ supersede").
    """
    old = db.get(Memory, old_id)
    if not old or old.owner_id != owner:
        raise ValueError("Old memory not found")
    if not new_id:
        raise ValueError(
            "supersede requires the replacement memory (new_id); use service.change(..., delete=True) to forget outright"
        )
    new = db.get(Memory, new_id)
    if not new or new.owner_id != owner:
        raise ValueError("Replacement memory not found")
    if old.id == new.id:
        raise ValueError("a memory cannot supersede itself")
    db.add(
        MemoryRevision(
            owner_id=owner,
            data={"memory_id": old.id, "previous": {"id": old.id, **legacy_service._trace_data(old)}},
        )
    )
    old.lifecycle_state = LIFECYCLE_SUPERSEDED
    old.superseded_by_id = new.id
    # A superseded row is settled, not contested.
    old.disputed_by_id = None
    db.flush()
    # §4: the `supersedes` edge mirrors `superseded_by_id` (and the cleared
    # `disputed_by_id` takes its edge with it) — same transaction, always.
    legacy_service.sync_links(db, old)
    _emit_event(db, owner, None, "SUPERSEDE", old.id, None, new.id, reason)
    return {"id": old_id, "superseded_by_id": new.id, "lifecycle": LIFECYCLE_SUPERSEDED}


def dispute(db, owner: str, *, memory_id: str, conflicting_id: str, reason: str = "") -> dict:
    """Spec §3: mark ``memory_id`` disputed against ``conflicting_id``.

    Only the named row is marked: it leaves retrieval (§3) while the memory it
    contradicts keeps serving recall — an unresolved conflict must not be able
    to hide the stored fact. ``disputed_by_id`` names the other side so
    :func:`resolve_conflict` can settle the pair later, and a revision of the
    pre-dispute state is written first (§1).
    """
    row = db.get(Memory, memory_id)
    other = db.get(Memory, conflicting_id)
    if not row or row.owner_id != owner:
        raise ValueError("Memory not found")
    if not other or other.owner_id != owner:
        raise ValueError("Conflicting memory not found")
    if row.id == other.id:
        raise ValueError("a memory cannot conflict with itself")
    db.add(
        MemoryRevision(
            owner_id=owner,
            data={"memory_id": row.id, "previous": {"id": row.id, **legacy_service._trace_data(row)}},
        )
    )
    row.lifecycle_state = LIFECYCLE_DISPUTED
    row.disputed_by_id = other.id
    db.flush()
    # §4: `contradicts` mirrors `disputed_by_id` on the disputed row (the
    # challenger row itself is untouched, exactly as §3 says).
    legacy_service.sync_links(db, row)
    _emit_event(db, owner, None, "DISPUTE", row.id, None, other.id, reason)
    return {
        "id": row.id,
        "a": row.id,
        "b": other.id,
        "disputed_by_id": other.id,
        "lifecycle": LIFECYCLE_DISPUTED,
    }


def resolve_conflict(db, owner: str, *, conflict_id: str, resolution: str, prefer: str | None = None) -> dict:
    """Spec §3 settlement, delegated to ``service.resolve_conflict``.

    ``conflict_id`` is the disputed row; its ``disputed_by_id`` names the
    other side. ``prefer`` is the id that wins and must be one of those two.
    Given no preference the *challenger does not win by default*: the memory
    the dispute points at (the one already stored) is adopted.
    """
    row = db.get(Memory, conflict_id)
    if not row or row.owner_id != owner:
        raise ValueError("Memory not found")
    other_id = row.disputed_by_id
    if not other_id:
        raise ValueError("Memory is not disputed")
    if prefer is not None and prefer not in (row.id, other_id):
        raise ValueError("prefer must be one of the two conflicting memories")
    winner_id = row.id if prefer == row.id else other_id
    loser_id = other_id if winner_id == row.id else row.id
    return legacy_service.resolve_conflict(
        db, owner, winner_id=winner_id, loser_id=loser_id, resolution=resolution
    )


def attach_evidence(
    db,
    owner: str,
    memory_id: str,
    *,
    kind: str = "observation",
    ref: str = "",
    summary: str = "",
    data=None,
    captured_at=None,
    confidence=None,
) -> dict:
    row = db.get(Memory, memory_id)
    if not row or row.owner_id != owner:
        raise ValueError("Memory not found")
    row.updated_at = now()
    return {"id": memory_id, "memory_id": memory_id, "kind": kind}


def group_for_context(memories: list[dict]) -> dict[str, list[dict]]:
    grouped: dict[str, list[dict]] = {
        "all": list(memories or []),
        "decisions": [],
        "failures": [],
        "facts": list(memories or []),
        "preferences": [],
        "goals": [],
        "questions": [],
        "experiences": [],
        "other": [],
    }
    return grouped


def compact_for_context(memories: list[dict], token_budget: int = 1500) -> list[dict]:
    if not memories:
        return []
    keep: list[dict] = []
    used = 0
    for mem in memories:
        compact = {"id": mem.get("id"), "content": str(mem.get("content", ""))[:ITEM_CHAR_LIMIT]}
        cost = len(str(compact)) // 2 + 20
        if used + cost > token_budget and keep:
            continue
        keep.append(compact)
        used += cost
        if used >= token_budget:
            break
    return keep


def _next_candidate_index(db, run_id: str | None) -> int:
    if not run_id:
        return 0
    stored = db.scalar(
        select(func.count()).select_from(MemoryActionEvent).where(MemoryActionEvent.run_id == run_id)
    )
    pending = sum(1 for obj in db.new if isinstance(obj, MemoryActionEvent) and obj.run_id == run_id)
    return int(stored or 0) + pending


def _emit_event(
    db, owner: str, run_id: str | None, action: str, memory_id: str | None, before, after, reason: str
) -> None:
    # `after` is the id the action moved *to* (§3 transitions name a second
    # row: SUPERSEDE's replacement, DISPUTE's counterpart, RESOLVE's winner).
    # Recording it means a two-row transition can be read back from the event
    # log alone, without re-deriving it from the rows.
    related = after if isinstance(after, str) and after else None
    db.add(
        MemoryActionEvent(
            owner_id=owner,
            run_id=run_id,
            candidate_index=_next_candidate_index(db, run_id),
            data={
                "action": action,
                "memory_id": memory_id,
                "related_memory_id": related,
                "reason": (reason or "")[:400],
                "runtime": "chatgpt_flat",
            },
        )
    )
