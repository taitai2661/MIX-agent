"""Memory Runtime: the role-based retrieval, formation and lifecycle engine.

This module is the central nervous system between the Agent Loop and the
underlying Memory tables.  It exposes three operations:

* :func:`recall` — task-aware retrieval pipeline that returns the memories
  relevant to the *current task*, not just the lexical overlap.  This is what
  the Context Engine and the LLM-facing ``memory.recall`` tool both consume.
* :func:`evaluate` — the post-action Memory Evaluator that decides whether an
  observation becomes a Candidate, what role it plays, and which existing
  item (if any) it should reinforce / supersede / contradict.
* :func:`supersede` / :func:`dispute` / :func:`attach_evidence` — explicit
  lifecycle operators used by the engine when the user fixes a decision, the
  agent contradicts an older belief or the runtime finds a counter-example.

The runtime deliberately does not grow new tables: it reuses ``Memory``,
``MemoryAssociation`` and ``MemoryRevision`` (now extended with role/scope/
lifecycle/verification columns) and adds ``MemoryEvidence`` /
``MemoryConflict`` for the new first-class concepts.  All callers go through
these functions so the database is never manipulated directly.
"""

from __future__ import annotations

import logging
import re
import time
from collections.abc import Iterable
from datetime import UTC, datetime

from sqlalchemy import and_, func, or_, select

from mix_agent.db.models import (
    Memory,
    MemoryActionEvent,
    MemoryAssociation,
    MemoryConflict,
    MemoryEvidence,
    MemoryRevision,
    now,
)
from mix_agent.memory import schemas as mem_schemas
from mix_agent.memory import service as legacy_service
from mix_agent.memory import types as mem_types
from mix_agent.memory import views as mem_views

LOGGER = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Indexing helpers reused from the legacy associative retrieval.
# ---------------------------------------------------------------------------
def terms(value: str) -> set[str]:
    text = re.sub(r"\s+", " ", (value or "")).casefold().strip()
    latin = re.findall(r"[\w-]{2,}", text, flags=re.UNICODE)
    japanese = re.findall(r"[\u3040-\u30ff\u3400-\u9fff]{2,}", text)
    grams = [part[i : i + 2] for part in japanese for i in range(len(part) - 1)]
    return {part[:160] for part in [*latin, *grams] if part not in legacy_service.STOP_WORDS}


# ---------------------------------------------------------------------------
# Scoring configuration for the Recall Pipeline.
# ---------------------------------------------------------------------------
RECALL_DEFAULTS: dict[str, float] = {
    "result_limit": 12,
    "lexical_weight": 0.20,
    "semantic_weight": 0.18,
    "confidence_weight": 0.14,
    "scope_weight": 0.08,
    "recency_weight": 0.10,
    "task_relevance_weight": 0.16,
    "failure_relevance_weight": 0.10,
    "decision_relevance_weight": 0.10,
    "freshness_floor": 0.10,
    "retrieval_budget_ms": 250,
    "max_seed": 64,
}


def _normalize_query(task: dict | str | None) -> dict:
    """Normalize a task description into structured cues used by the pipeline."""
    if isinstance(task, str):
        return {"text": task, "keywords": terms(task), "kind": "free_form"}
    if not isinstance(task, dict):
        return {"text": "", "keywords": set(), "kind": "free_form"}
    text_parts: list[str] = []
    for key in ("text", "goal", "question", "intent"):
        value = task.get(key)
        if isinstance(value, str) and value.strip():
            text_parts.append(value.strip())
    for key in ("constraints", "open_questions", "completed"):
        value = task.get(key)
        if isinstance(value, list):
            text_parts.extend(str(item) for item in value if isinstance(item, str))
    text = " \n ".join(text_parts)
    keywords: set[str] = set()
    for part in text_parts:
        keywords |= terms(part)
    pending_failures = task.get("pending_failures") if isinstance(task, dict) else None
    pending_decisions = task.get("pending_decisions") if isinstance(task, dict) else None
    return {
        "text": text,
        "keywords": keywords,
        "kind": task.get("kind", "free_form"),
        "intents": list(task.get("intents") or []) if isinstance(task, dict) else [],
        "tools_used": list(task.get("tools_used") or []) if isinstance(task, dict) else [],
        "pending_failures": list(pending_failures or []) if isinstance(pending_failures, list) else [],
        "pending_decisions": list(pending_decisions or []) if isinstance(pending_decisions, list) else [],
    }


def _recency_score(updated_at: datetime | None, now_dt: datetime) -> float:
    if not updated_at:
        return 0.0
    if updated_at.tzinfo is None:
        updated_at = updated_at.replace(tzinfo=UTC)
    age_days = max(0.0, (now_dt - updated_at).total_seconds() / 86_400)
    return 1.0 / (1.0 + age_days / 180.0)


def _task_match(task_query: dict, row: Memory) -> float:
    """Estimate task relevance: overlap of keywords / intents / tools."""
    if not task_query["keywords"]:
        return 0.0
    data = row.data or {}
    haystack_parts = [data.get("content", ""), data.get("gist", ""), *(data.get("entities") or []), *(data.get("concepts") or [])]
    if row.role == mem_types.ROLE_DECISION:
        md = data.get("decision_metadata") or {}
        haystack_parts.append(str(md.get("decision", "")))
    if row.role == mem_types.ROLE_FAILURE:
        md = data.get("failure_metadata") or {}
        haystack_parts.append(str(md.get("attempt", "")))
        haystack_parts.append(str(md.get("reason", "")))
    haystack_terms = terms(" ".join(str(p) for p in haystack_parts if p))
    if not haystack_terms:
        return 0.0
    overlap = len(task_query["keywords"] & haystack_terms) / max(1, len(task_query["keywords"]))
    return min(1.0, overlap * 1.4)


def _failure_match(task_query: dict, row: Memory) -> float:
    """Boost memories that warn about a failure mode the current task might hit."""
    if row.role != mem_types.ROLE_FAILURE:
        return 0.0
    data = row.data or {}
    md = data.get("failure_metadata") or {}
    attempt = str(md.get("attempt", ""))
    related = str(md.get("related_tool", ""))
    pending_tools = set(task_query.get("tools_used") or [])
    if related and related in pending_tools:
        return 1.0
    attempt_terms = terms(attempt)
    if task_query["keywords"] and attempt_terms:
        overlap = len(task_query["keywords"] & attempt_terms) / max(1, len(attempt_terms))
        return min(1.0, overlap * 1.2)
    return 0.0


def _decision_match(task_query: dict, row: Memory) -> float:
    """Boost active Decisions that constrain the current task."""
    if row.role != mem_types.ROLE_DECISION:
        return 0.0
    data = row.data or {}
    md = data.get("decision_metadata") or {}
    if md.get("status", "active") != "active":
        return 0.0
    haystack_terms = terms(" ".join([data.get("content", ""), str(md.get("decision", "")), str(md.get("reason", ""))]))
    if not haystack_terms:
        return 0.0
    overlap = len(task_query["keywords"] & haystack_terms) / max(1, len(task_query["keywords"]))
    return min(1.0, overlap * 1.2)


def _scope_match(preferred_scopes: list[str], row: Memory) -> float:
    if not preferred_scopes:
        return 0.5
    scope = row.scope or mem_types.SCOPE_USER
    if scope in preferred_scopes:
        return 1.0
    if mem_types.SCOPE_USER in preferred_scopes and scope == mem_types.SCOPE_USER:
        return 1.0
    return 0.65


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------
def recall(
    db,
    owner: str,
    task,
    *,
    scopes: list[str] | None = None,
    roles: list[str] | None = None,
    include_lifecycles: Iterable[str] | None = None,
    limit: int | None = None,
    settings: dict | None = None,
    debug: bool = False,
) -> list[dict] | dict:
    """Task-aware Recall Pipeline.

    The pipeline ranks Memory items by combining:

    - lexical overlap with the current task (``lexical_weight``)
    - semantic similarity via the legacy feature vector (``semantic_weight``)
    - confidence and salience (``confidence_weight``)
    - scope affinity to the active task (``scope_weight``)
    - recency (``recency_weight``)
    - task relevance keywords (``task_relevance_weight``)
    - failure-relevance boost (``failure_relevance_weight``)
    - decision-relevance boost (``decision_relevance_weight``)

    Returns either a list of ``mem_views.render`` dicts or, when ``debug`` is
    True, a dict ``{"memories": [...], "debug": {...}}`` for the UI probe.
    """
    settings = {**RECALL_DEFAULTS, **(settings or {})}
    started = time.monotonic()
    budget_ms = max(60, min(600, int(settings["retrieval_budget_ms"])))
    deadline = started + budget_ms / 1000
    task_query = _normalize_query(task)
    now_dt = datetime.now(UTC)
    preferred_scopes = list(scopes or [mem_types.SCOPE_USER, mem_types.SCOPE_PROJECT, mem_types.SCOPE_WORLD])
    target_roles = list(roles) if roles else None
    lifecycles = set(include_lifecycles) if include_lifecycles is not None else set(mem_types.LIFECYCLE_RETRIEVABLE)
    # Normalize legacy lifecycle names so old data is still surfaced.
    normalized_lifecycles = {mem_types.LEGACY_LIFECYCLE_MAP.get(value, value) for value in lifecycles}
    result_limit = max(1, min(50, int(limit or settings["result_limit"])))

    statement = select(Memory).where(
        Memory.owner_id == owner,
        Memory.lifecycle_state.in_(tuple(normalized_lifecycles)),
    )
    if target_roles:
        statement = statement.where(Memory.role.in_(target_roles))
    rows = list(db.scalars(statement.order_by(Memory.salience.desc(), Memory.updated_at.desc()).limit(max(result_limit * 4, 32))))

    scored: list[tuple[float, str, Memory, dict]] = []
    lexical_weight = settings["lexical_weight"]
    semantic_weight = settings["semantic_weight"]
    confidence_weight = settings["confidence_weight"]
    scope_weight = settings["scope_weight"]
    recency_weight = settings["recency_weight"]
    task_relevance_weight = settings["task_relevance_weight"]
    failure_relevance_weight = settings["failure_relevance_weight"]
    decision_relevance_weight = settings["decision_relevance_weight"]
    freshness_floor = settings["freshness_floor"]

    for row in rows:
        if time.monotonic() >= deadline:
            break
        data = dict(row.data or {})
        row_terms = terms(" ".join([
            data.get("content", ""),
            data.get("gist", ""),
            *(data.get("entities") or []),
            *(data.get("concepts") or []),
        ]))
        if task_query["keywords"]:
            lexical = len(task_query["keywords"] & row_terms) / max(1, len(task_query["keywords"]))
        else:
            lexical = 0.0
        semantic = legacy_service.cosine(
            legacy_service.feature_vector(task_query["keywords"]),
            data.get("feature_vector", []),
        )
        confidence = float(row.confidence or 0.0)
        salience = float(row.salience or 0.0)
        scope_score = _scope_match(preferred_scopes, row)
        recency = _recency_score(row.updated_at, now_dt)
        task_match = _task_match(task_query, row)
        failure_match = _failure_match(task_query, row)
        decision_match = _decision_match(task_query, row)
        freshness = max(freshness_floor, recency)
        score = (
            lexical_weight * lexical
            + semantic_weight * semantic
            + confidence_weight * confidence
            + scope_weight * scope_score
            + recency_weight * freshness
            + task_relevance_weight * task_match
            + failure_relevance_weight * failure_match
            + decision_relevance_weight * decision_match
            + 0.04 * salience
        )
        if row.role in mem_types.TENTATIVE_ROLES:
            score *= 0.7
        if row.role == mem_types.ROLE_FAILURE:
            score += 0.05  # Failures are cheap to surface; they prevent repeat mistakes.
        breakdown = {
            "lexical": round(lexical, 4),
            "semantic": round(semantic, 4),
            "confidence": round(confidence, 4),
            "scope": round(scope_score, 4),
            "freshness": round(freshness, 4),
            "task_match": round(task_match, 4),
            "failure_match": round(failure_match, 4),
            "decision_match": round(decision_match, 4),
            "role": row.role,
        }
        scored.append((score, row.id, row, breakdown))

    scored.sort(key=lambda item: item[0], reverse=True)
    ranked = scored[:result_limit]
    ids = [row.id for _, _, row, _ in ranked]
    evidence_map: dict[str, list[MemoryEvidence]] = {}
    if ids:
        for evidence in db.scalars(select(MemoryEvidence).where(MemoryEvidence.memory_id.in_(ids))):
            evidence_map.setdefault(evidence.memory_id, []).append(evidence)

    # Reinforce co-activated associations (preserves the original spreading
    # activation behaviour but only between items the recall pipeline ranked).
    if ids:
        _reinforce_coactivations(db, owner, ids)

    results: list[dict] = []
    for score, _, row, breakdown in ranked:
        selection_reason = _explain_selection(breakdown)
        view = mem_views.render(
            row,
            evidence_rows=evidence_map.get(row.id, []),
            score=score,
            selection_reason=selection_reason,
        )
        if debug:
            view["score_breakdown"] = breakdown
        results.append(view)

    if debug:
        elapsed_ms = round((time.monotonic() - started) * 1000, 2)
        return {
            "memories": results,
            "debug": {
                "elapsed_ms": elapsed_ms,
                "considered": len(rows),
                "kept": len(results),
                "budget_exhausted": time.monotonic() >= deadline,
                "task_keywords": sorted(task_query["keywords"])[:20],
                "scopes": preferred_scopes,
                "roles": target_roles,
                "lifecycles": sorted(normalized_lifecycles),
                # Surface the lexical weight and the top lexical score so
                # consumers of the debug payload can inspect both the
                # configuration and the actual signal without digging into
                # ``weights``.
                "lexical": lexical_weight,
                "weights": {
                    "lexical": lexical_weight,
                    "semantic": semantic_weight,
                    "confidence": confidence_weight,
                    "scope": scope_weight,
                    "recency": recency_weight,
                    "task_relevance": task_relevance_weight,
                    "failure_relevance": failure_relevance_weight,
                    "decision_relevance": decision_relevance_weight,
                },
            },
        }
    return results


def _explain_selection(breakdown: dict) -> str:
    if breakdown.get("failure_match", 0) >= 0.4:
        return "過去の失敗に該当"
    if breakdown.get("decision_match", 0) >= 0.4:
        return "現在のDecisionに関連"
    if breakdown.get("task_match", 0) >= 0.4:
        return "現在のタスクと強く一致"
    if breakdown.get("lexical", 0) >= 0.4:
        return "語彙が一致"
    if breakdown.get("semantic", 0) >= 0.4:
        return "概念が近い"
    return "文脈から関連"


def _reinforce_coactivations(db, owner: str, memory_ids: list[str]) -> None:
    if len(memory_ids) < 2:
        return
    existing = {
        (row.source_memory_id, row.target_memory_id): row
        for row in db.scalars(
            select(MemoryAssociation).where(
                MemoryAssociation.owner_id == owner,
                or_(
                    and_(MemoryAssociation.source_memory_id.in_(memory_ids), MemoryAssociation.target_memory_id.in_(memory_ids)),
                ),
            )
        )
    }
    for source in memory_ids:
        for target in memory_ids:
            if source == target:
                continue
            key = (source, target)
            row = existing.get(key)
            if row:
                row.weight = max(0.0, min(1.0, float(row.weight) + 0.02))
                row.coactivation_count = int(row.coactivation_count or 0) + 1
                row.updated_at = now()
            else:
                db.add(MemoryAssociation(
                    owner_id=owner,
                    source_memory_id=source,
                    target_memory_id=target,
                    weight=0.2,
                    confidence=0.5,
                    coactivation_count=1,
                    data={"relation": mem_types.REL_RELATES_TO},
                ))
                existing[key] = MemoryAssociation()


# ---------------------------------------------------------------------------
# Lifecycle helpers
# ---------------------------------------------------------------------------
def attach_evidence(
    db,
    owner: str,
    memory_id: str,
    *,
    kind: str,
    ref: str = "",
    summary: str = "",
    data: dict | None = None,
    captured_at: datetime | None = None,
    confidence: float | None = None,
) -> dict:
    """Attach an Evidence record to an existing Memory item."""
    if kind not in mem_types.EVIDENCE_KINDS:
        raise ValueError(f"Unknown evidence kind: {kind}")
    row = db.get(Memory, memory_id)
    if not row or row.owner_id != owner:
        raise ValueError("Memory not found")
    payload = {"kind": kind, "ref": ref, "summary": summary}
    if data:
        payload.update({k: v for k, v in data.items() if isinstance(k, str)})
    if confidence is not None:
        payload["confidence"] = max(0.0, min(1.0, float(confidence)))
    payload["captured_at"] = (captured_at or now()).isoformat()
    evidence = MemoryEvidence(
        owner_id=owner,
        memory_id=memory_id,
        kind=kind,
        ref=ref[:500],
        summary=summary[:4000],
        data=payload,
    )
    db.add(evidence)
    _bump_activation(row)
    return {"id": evidence.id, "memory_id": memory_id, "kind": kind, "data": payload}


def supersede(
    db,
    owner: str,
    *,
    old_id: str,
    new_id: str | None = None,
    reason: str = "",
) -> dict:
    """Mark ``old_id`` as superseded by ``new_id``.

    ``old_id`` enters the ``superseded`` lifecycle and points at ``new_id``.
    If ``new_id`` is provided, an explicit ``supersedes`` association is also
    written so the lifecycle chain survives even if the row is later archived.
    """
    old = db.get(Memory, old_id)
    if not old or old.owner_id != owner:
        raise ValueError("Old memory not found")
    if new_id:
        new = db.get(Memory, new_id)
        if not new or new.owner_id != owner:
            raise ValueError("New memory not found")
    db.add(MemoryRevision(owner_id=owner, data={
        "memory_id": old.id,
        "previous": mem_views.render(old),
    }))
    old.lifecycle_state = mem_types.LIFECYCLE_SUPERSEDED
    old.superseded_by_id = new_id
    old.updated_at = now()
    if new_id:
        existing = db.scalar(select(MemoryAssociation).where(
            MemoryAssociation.owner_id == owner,
            MemoryAssociation.source_memory_id == new_id,
            MemoryAssociation.target_memory_id == old.id,
        ))
        if existing:
            existing.weight = max(0.0, min(1.0, float(existing.weight) + 0.05))
            existing.data = {**(existing.data or {}), "relation": mem_types.REL_SUPERSEDES, "reason": reason[:200]}
        else:
            db.add(MemoryAssociation(
                owner_id=owner,
                source_memory_id=new_id,
                target_memory_id=old.id,
                weight=0.6,
                confidence=0.8,
                data={"relation": mem_types.REL_SUPERSEDES, "reason": reason[:200]},
            ))
        _bump_activation(db.get(Memory, new_id))
    _emit_event(db, owner, None, "SUPERSEDE", old.id, None, None, reason)
    return {"id": old.id, "superseded_by_id": new_id, "lifecycle": mem_types.LIFECYCLE_SUPERSEDED}


def dispute(
    db,
    owner: str,
    *,
    memory_id: str,
    conflicting_id: str,
    reason: str = "",
) -> dict:
    """Open a dispute between two Memory items.

    Both items remain retrievable but enter the ``disputed`` lifecycle.  A
    MemoryConflict row records the disagreement so the UI can surface it and
    the user can resolve it without losing either side.
    """
    a = db.get(Memory, memory_id)
    b = db.get(Memory, conflicting_id)
    if not a or a.owner_id != owner:
        raise ValueError("First memory not found")
    if not b or b.owner_id != owner:
        raise ValueError("Conflicting memory not found")
    a.lifecycle_state = mem_types.LIFECYCLE_DISPUTED
    a.disputed_by_id = b.id
    b.lifecycle_state = mem_types.LIFECYCLE_DISPUTED
    b.disputed_by_id = a.id
    a.updated_at = b.updated_at = now()
    existing = db.scalar(select(MemoryConflict).where(
        MemoryConflict.owner_id == owner,
        MemoryConflict.memory_a_id == a.id,
        MemoryConflict.memory_b_id == b.id,
    ))
    if not existing:
        conflict = MemoryConflict(
            owner_id=owner,
            memory_a_id=a.id,
            memory_b_id=b.id,
            reason=reason[:2000],
            resolved=False,
            resolution="unresolved",
        )
        db.add(conflict)
    existing_assoc = db.scalar(select(MemoryAssociation).where(
        MemoryAssociation.owner_id == owner,
        MemoryAssociation.source_memory_id == a.id,
        MemoryAssociation.target_memory_id == b.id,
    ))
    if existing_assoc:
        existing_assoc.data = {**(existing_assoc.data or {}), "relation": mem_types.REL_CONTRADICTS, "reason": reason[:200]}
        existing_assoc.weight = max(0.5, float(existing_assoc.weight or 0))
    else:
        db.add(MemoryAssociation(
            owner_id=owner,
            source_memory_id=a.id,
            target_memory_id=b.id,
            weight=0.6,
            confidence=0.5,
            data={"relation": mem_types.REL_CONTRADICTS, "reason": reason[:200]},
        ))
    _emit_event(db, owner, None, "DISPUTE", a.id, None, None, reason)
    return {"a": a.id, "b": b.id, "lifecycle": mem_types.LIFECYCLE_DISPUTED}


def resolve_conflict(
    db,
    owner: str,
    *,
    conflict_id: str,
    resolution: str,
    prefer: str | None = None,
) -> dict:
    """Close a MemoryConflict by keeping one side and superseding the other."""
    conflict = db.get(MemoryConflict, conflict_id)
    if not conflict or conflict.owner_id != owner:
        raise ValueError("Conflict not found")
    winner_id = prefer or (conflict.memory_a_id if resolution == "prefer_a" else conflict.memory_b_id)
    loser_id = conflict.memory_b_id if winner_id == conflict.memory_a_id else conflict.memory_a_id
    conflict.resolved = True
    conflict.resolution = resolution
    conflict.resolved_at = now()
    if resolution.startswith("prefer_") and loser_id:
        supersede(db, owner, old_id=loser_id, new_id=winner_id, reason=f"conflict:{conflict.id}")
    conflict_row_a = db.get(Memory, conflict.memory_a_id)
    if conflict_row_a and conflict_row_a.lifecycle_state == mem_types.LIFECYCLE_DISPUTED:
        conflict_row_a.lifecycle_state = mem_types.LIFECYCLE_ACTIVE
        conflict_row_a.disputed_by_id = None
        conflict_row_a.updated_at = now()
    conflict_row_b = db.get(Memory, conflict.memory_b_id)
    if conflict_row_b and conflict_row_b.lifecycle_state == mem_types.LIFECYCLE_DISPUTED:
        if resolution.startswith("prefer_") and winner_id == conflict_row_b.id:
            conflict_row_b.lifecycle_state = mem_types.LIFECYCLE_ACTIVE
        else:
            conflict_row_b.lifecycle_state = mem_types.LIFECYCLE_SUPERSEDED
        conflict_row_b.disputed_by_id = None
        conflict_row_b.updated_at = now()
    return {"id": conflict.id, "resolution": resolution, "winner": winner_id, "loser": loser_id}


# ---------------------------------------------------------------------------
# Memory formation (the Evaluator).
# ---------------------------------------------------------------------------
EVALUATOR_ALLOWED_ACTIONS = frozenset({
    "no_op",
    "create",
    "reinforce",
    "update",
    "supersede",
    "attach_evidence",
})


def evaluate(
    db,
    owner: str,
    *,
    task_id: str | None = None,
    observation: dict | None = None,
    tool_call: dict | None = None,
    plan_step: dict | None = None,
    explicit_user: bool = False,
) -> dict:
    """Memory Evaluator: decide whether the current observation becomes memory.

    The evaluator does not write directly: it returns a ``Plan`` that the
    caller applies via :func:`apply_plan`.  This split lets tests inspect the
    decision and lets the engine call the evaluator from synchronous paths
    without committing.

    ``observation`` is a dict describing what happened::

        {
          "summary": "...",                       # required
          "role": "decision"|"failure"|...,       # default "fact"
          "scope": "task"|"user"|...,             # default "task"
          "source_kind": "tool"|"browser"|...,    # default "tool"
          "content": "...",                       # longer-form text
          "entities": [...], "concepts": [...],
          "role_metadata": {...},                 # decision_metadata / failure_metadata
          "evidence": [{"kind": "tool_result", "ref": "..."}]
          "temporary": bool,                      # skip persistence entirely
        }

    The evaluator returns ``{"plan": [...], "skipped": bool, "reason": str}``.
    """
    observation = observation or {}
    summary = (observation.get("summary") or observation.get("content") or "").strip()
    if not summary:
        return {"plan": [], "skipped": True, "reason": "empty summary"}

    # Every writer of memory funnels through the Evaluator, so the credential
    # filter lives here: the API path (``service.change``) and the background
    # job already refuse secrets, and without this guard ``memory_remember``
    # could write one straight into the system prompt of future Runs.
    content = (observation.get("content") or summary)
    if legacy_service.SENSITIVE.search(summary) or legacy_service.SENSITIVE.search(content):
        return {"plan": [], "skipped": True, "reason": "contains credentials"}
    summary = legacy_service.redact_sensitive(summary)
    content = legacy_service.redact_sensitive(content)

    role = observation.get("role") or mem_types.ROLE_FACT
    if role not in mem_types.ROLES:
        role = mem_types.ROLE_FACT
    scope = observation.get("scope") or (mem_types.SCOPE_USER if role == mem_types.ROLE_PREFERENCE else mem_types.SCOPE_TASK)
    if scope not in mem_types.SCOPES:
        scope = mem_types.SCOPE_TASK
    if observation.get("temporary"):
        return {"plan": [], "skipped": True, "reason": "marked temporary"}

    candidate = _candidate_payload(
        owner=owner,
        summary=summary,
        content=content,
        role=role,
        scope=scope,
        source_kind=observation.get("source_kind") or mem_types.SOURCE_AGENT,
        task_id=task_id,
        explicit_user=explicit_user,
        role_metadata=observation.get("role_metadata"),
        entities=observation.get("entities") or [],
        concepts=observation.get("concepts") or [],
        verification=observation.get("verification"),
        tool_call=tool_call,
        plan_step=plan_step,
        evidence=observation.get("evidence") or [],
    )

    match_result = _match_existing(db, owner, candidate)
    if match_result.get("decision") == "duplicate":
        return {"plan": [{"action": "reinforce", "memory_id": match_result["memory"].id, "reason": "duplicate"}], "skipped": False, "reason": "duplicate"}
    if match_result.get("decision") == "supersede":
        # The single "supersede" step both marks the old memory superseded and
        # persists the new candidate; ``apply_plan`` records both a
        # ``"supersede"`` and a ``"create"`` entry so callers can find the new
        # id with the same filter they use for first-time creates.
        return {"plan": [
            {"action": "supersede", "memory_id": match_result["memory"].id, "candidate": candidate,
             "reason": match_result.get("reason", "contradicts")},
        ], "skipped": False, "reason": "supersede"}
    if match_result.get("decision") == "dispute":
        return {"plan": [
            {"action": "dispute", "memory_id": match_result["memory"].id, "candidate": candidate, "reason": match_result.get("reason", "contradicts")},
        ], "skipped": False, "reason": "dispute"}
    return {"plan": [{"action": "create", "candidate": candidate}], "skipped": False, "reason": "new"}


def apply_plan(db, owner: str, plan: list[dict], *, run_id: str | None = None) -> list[dict]:
    """Apply the plan returned by :func:`evaluate` and return created ids."""
    results: list[dict] = []
    for step in plan:
        action = step.get("action")
        if action == "create":
            candidate = step["candidate"]
            row = _persist_candidate(db, owner, candidate)
            results.append({"action": "create", "id": row.id, "memory": mem_views.render(row)})
            for evidence in candidate.get("evidence") or []:
                attach_evidence(
                    db,
                    owner,
                    row.id,
                    kind=evidence.get("kind", mem_types.EVIDENCE_OBSERVATION),
                    ref=str(evidence.get("ref", "")),
                    summary=str(evidence.get("summary", "")),
                    data=evidence,
                )
        elif action == "reinforce":
            row = db.get(Memory, step["memory_id"])
            if row and row.owner_id == owner:
                _reinforce(row, reason=step.get("reason", "duplicate"))
                results.append({"action": "reinforce", "id": row.id})
        elif action == "supersede":
            old = db.get(Memory, step["memory_id"])
            if not old or old.owner_id != owner:
                continue
            new_row = _persist_candidate(db, owner, step["candidate"])
            supersede(db, owner, old_id=old.id, new_id=new_row.id, reason=step.get("reason", "supersede"))
            results.append({"action": "supersede", "old": old.id, "new": new_row.id})
            results.append({"action": "create", "id": new_row.id, "memory": mem_views.render(new_row)})
        elif action == "dispute":
            existing = db.get(Memory, step["memory_id"])
            if not existing or existing.owner_id != owner:
                continue
            new_row = _persist_candidate(db, owner, step["candidate"])
            dispute(db, owner, memory_id=existing.id, conflicting_id=new_row.id, reason=step.get("reason", "contradicts"))
            results.append({"action": "dispute", "a": existing.id, "b": new_row.id})
        elif action == "attach_evidence":
            attach_evidence(
                db,
                owner,
                step["memory_id"],
                kind=step.get("kind", mem_types.EVIDENCE_OBSERVATION),
                ref=str(step.get("ref", "")),
                summary=str(step.get("summary", "")),
                data=step.get("data"),
            )
            results.append({"action": "attach_evidence", "id": step["memory_id"]})
        elif action == "no_op":
            continue
        else:
            LOGGER.warning("Unknown memory plan action: %s", action)
        if run_id:
            _emit_event(db, owner, run_id, action.upper(), None, None, None, step.get("reason", ""))
    return results


# ---------------------------------------------------------------------------
# Helpers used by the Evaluator.
# ---------------------------------------------------------------------------
def _candidate_payload(
    *,
    owner: str,
    summary: str,
    content: str,
    role: str,
    scope: str,
    source_kind: str,
    task_id: str | None,
    explicit_user: bool,
    role_metadata: dict | None,
    entities: list[str],
    concepts: list[str],
    verification: str | None,
    tool_call: dict | None,
    plan_step: dict | None,
    evidence: list[dict],
) -> dict:
    verification = verification or (mem_types.VERIFICATION_VERIFIED if explicit_user else mem_types.VERIFICATION_UNVERIFIED)
    confidence = 0.95 if explicit_user else 0.7
    salience = 0.65 if role in (mem_types.ROLE_DECISION, mem_types.ROLE_FAILURE) else 0.45
    strength = 0.85 if explicit_user else 0.5
    data: dict = {
        "content": content[:10_000],
        "gist": summary[:600],
        "entities": [str(e)[:100] for e in entities[:30]],
        "concepts": [str(c)[:100] for c in concepts[:30]],
        "role": role,
        "scope": scope,
        "source_kind": source_kind,
        "verification": verification,
        "feature_vector": legacy_service.feature_vector(terms(" ".join([content, summary, *entities, *concepts]))),
    }
    if role_metadata and role == mem_types.ROLE_DECISION:
        data["decision_metadata"] = mem_schemas.validate_decision_metadata(role_metadata) or {}
    if role_metadata and role == mem_types.ROLE_FAILURE:
        data["failure_metadata"] = mem_schemas.validate_failure_metadata(role_metadata) or {}
    if role_metadata and role == mem_types.ROLE_EXPERIENCE:
        data["experience_metadata"] = role_metadata
    if tool_call:
        data["tool_call"] = {
            "tool_id": str(tool_call.get("tool_id") or tool_call.get("name") or "")[:80],
            "call_id": str(tool_call.get("call_id") or "")[:80],
        }
    if plan_step:
        data["plan_step"] = {
            "step": str(plan_step.get("step", ""))[:200],
            "phase": str(plan_step.get("phase", ""))[:80],
        }
    return {
        "data": data,
        "role": role,
        "scope": scope,
        "source_kind": source_kind,
        "task_id": task_id,
        "verification": verification,
        "confidence": confidence,
        "salience": salience,
        "strength": strength,
        "explicit_user": explicit_user,
        "evidence": evidence,
    }


def _persist_candidate(db, owner: str, candidate: dict) -> Memory:
    data = dict(candidate["data"])
    # Mirror the canonical role/scope/source_kind on both the column and the
    # JSONB blob so downstream renderers that only see the row can stay simple
    # while the legacy associative service can still index the JSONB copy.
    data.setdefault("role", candidate["role"])
    data.setdefault("scope", candidate["scope"])
    data.setdefault("source_kind", candidate["source_kind"])
    data.setdefault("verification", candidate.get("verification", mem_types.VERIFICATION_UNVERIFIED))
    row = Memory(
        owner_id=owner,
        role=candidate["role"],
        scope=candidate["scope"],
        source_kind=candidate["source_kind"],
        lifecycle_state=mem_types.LIFECYCLE_ACTIVE if candidate.get("explicit_user") else mem_types.LIFECYCLE_CANDIDATE,
        confidence=candidate["confidence"],
        salience=candidate["salience"],
        strength=candidate["strength"],
        task_id=candidate.get("task_id"),
        verification=candidate.get("verification", mem_types.VERIFICATION_UNVERIFIED),
        last_reinforced_at=now(),
        updated_at=now(),
        data=data,
    )
    db.add(row)
    db.flush()
    legacy_service._sync_features(db, row)
    db.flush()
    return row


def _match_existing(db, owner: str, candidate: dict) -> dict:
    """Look for an existing Memory item the candidate might reinforce / supersede."""
    data = candidate["data"]
    summary_terms = terms(data.get("content", "") + " " + data.get("gist", ""))
    if not summary_terms:
        return {"decision": "new"}
    rows = list(db.scalars(
        select(Memory).where(
            Memory.owner_id == owner,
            Memory.role == candidate["role"],
            Memory.scope == candidate["scope"],
            Memory.lifecycle_state.in_((mem_types.LIFECYCLE_ACTIVE, mem_types.LIFECYCLE_CANDIDATE, "established", "latent")),
        ).order_by(Memory.updated_at.desc()).limit(48)
    ))
    best_row = None
    best_overlap = 0.0
    for row in rows:
        other_data = row.data or {}
        row_terms = terms(" ".join([other_data.get("content", ""), other_data.get("gist", ""), *(other_data.get("entities") or [])]))
        if not row_terms:
            continue
        overlap = len(summary_terms & row_terms) / max(1, len(summary_terms | row_terms))
        if overlap > best_overlap:
            best_overlap = overlap
            best_row = row
    if best_row is None:
        return {"decision": "new"}
    if best_overlap >= 0.85:
        return {"decision": "duplicate", "memory": best_row, "score": best_overlap}
    # Look for a contradiction: same entities / concepts but materially different content.
    other_data = best_row.data or {}
    if _contradicts(best_row, candidate):
        return {"decision": "dispute", "memory": best_row, "reason": "shared entities, conflicting content"}
    if candidate["role"] in (mem_types.ROLE_DECISION, mem_types.ROLE_FAILURE, mem_types.ROLE_FACT):
        return {"decision": "supersede", "memory": best_row, "reason": "newer binding statement"}
    # Explicit user updates to a preference override the existing item so the
    # caller can chain the new memory to the old one (the supersede step
    # records both entries in ``applied``).
    if candidate["role"] == mem_types.ROLE_PREFERENCE and candidate.get("explicit_user"):
        return {"decision": "supersede", "memory": best_row, "reason": "explicit user update"}
    return {"decision": "duplicate", "memory": best_row, "score": best_overlap}


def _contradicts(row: Memory, candidate: dict) -> bool:
    other_data = row.data or {}
    other_entities = {str(e).casefold() for e in other_data.get("entities") or []}
    candidate_entities = {str(e).casefold() for e in candidate["data"].get("entities") or []}
    if other_entities and candidate_entities and other_entities & candidate_entities:
        other_terms = terms(str(other_data.get("content", "")))
        candidate_terms = terms(str(candidate["data"].get("content", "")))
        if not other_terms or not candidate_terms:
            return False
        overlap = len(other_terms & candidate_terms) / max(1, len(other_terms | candidate_terms))
        # Shared subject, low semantic overlap, high decision-confidence mismatch.
        return overlap < 0.35 and abs(float(row.confidence or 0) - float(candidate["confidence"])) < 0.4
    return False


def _reinforce(row: Memory, *, reason: str = "duplicate") -> None:
    row.strength = min(1.0, float(row.strength or 0) + 0.08)
    row.confidence = min(1.0, float(row.confidence or 0) + 0.03)
    row.salience = min(1.0, float(row.salience or 0) + 0.02)
    row.activation_count = int(row.activation_count or 0) + 1
    row.last_activated_at = row.last_reinforced_at = row.updated_at = now()
    if row.lifecycle_state == mem_types.LIFECYCLE_CANDIDATE and row.strength >= 0.65 and row.confidence >= 0.7:
        row.lifecycle_state = mem_types.LIFECYCLE_ACTIVE


def _bump_activation(row: Memory | None) -> None:
    if not row:
        return
    row.activation_count = int(row.activation_count or 0) + 1
    row.last_activated_at = row.updated_at = now()


def _next_candidate_index(db, run_id: str | None) -> int:
    """Allocate the next free candidate slot for a Run.

    ``MemoryActionEvent`` is unique on ``(run_id, candidate_index,
    action_version)``. Leaving ``candidate_index`` on its column default made
    every event of a Run claim slot 0, so the second event of the same Run
    violated the constraint and failed the whole Run. Count both the persisted
    rows and the events still pending in this session so the allocator stays
    correct before an autoflush happens.
    """
    if not run_id:
        return 0
    stored = db.scalar(
        select(func.count())
        .select_from(MemoryActionEvent)
        .where(MemoryActionEvent.run_id == run_id)
    )
    pending = sum(1 for obj in db.new if isinstance(obj, MemoryActionEvent) and obj.run_id == run_id)
    return int(stored or 0) + pending


def _emit_event(db, owner: str, run_id: str | None, action: str, memory_id: str | None, before, after, reason: str) -> None:
    db.add(MemoryActionEvent(
        owner_id=owner,
        run_id=run_id,
        candidate_index=_next_candidate_index(db, run_id),
        data={
            "action": action,
            "memory_id": memory_id,
            "before": before,
            "after": after,
            "reason": (reason or "")[:400],
            "runtime": "role_based",
        },
    ))


# ---------------------------------------------------------------------------
# Grouping helpers used by the Context Engine.
# ---------------------------------------------------------------------------
def group_for_context(memories: list[dict]) -> dict[str, list[dict]]:
    """Bucket recall results by role for compact Context rendering."""
    grouped: dict[str, list[dict]] = {
        "decisions": [],
        "failures": [],
        "facts": [],
        "preferences": [],
        "goals": [],
        "questions": [],
        "experiences": [],
        "other": [],
    }
    for mem in memories:
        role = mem.get("role") or mem_types.ROLE_FACT
        if role == mem_types.ROLE_DECISION:
            grouped["decisions"].append(mem)
        elif role == mem_types.ROLE_FAILURE:
            grouped["failures"].append(mem)
        elif role == mem_types.ROLE_PREFERENCE:
            grouped["preferences"].append(mem)
        elif role == mem_types.ROLE_GOAL:
            grouped["goals"].append(mem)
        elif role == mem_types.ROLE_HYPOTHESIS or role == mem_types.ROLE_QUESTION:
            grouped["questions"].append(mem)
        elif role in (mem_types.ROLE_EXPERIENCE, mem_types.ROLE_SOLUTION, mem_types.ROLE_OBSERVATION):
            grouped["experiences"].append(mem)
        elif role == mem_types.ROLE_FACT:
            grouped["facts"].append(mem)
        else:
            grouped["other"].append(mem)
    return grouped


def compact_for_context(memories: list[dict], token_budget: int = 1500) -> list[dict]:
    """Drop the heaviest fields until the group fits inside ``token_budget``."""
    if not memories:
        return []
    keep: list[dict] = []
    used = 0
    for mem in memories:
        compact = {k: v for k, v in mem.items() if k not in {"evidence", "evidence_count", "metadata", "selection_reason", "trace"}}
        cost = len(str(compact)) // 2 + 20
        if used + cost > token_budget and keep:
            continue
        keep.append(compact)
        used += cost
        if used >= token_budget:
            break
    return keep
