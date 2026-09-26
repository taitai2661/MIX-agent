"""Public view rendering for Memory items.

The runtime stores raw Memory rows; every consumer (Context Engine, UI, debug
endpoints, Memory Tools) needs a normalized shape that exposes role, scope,
evidence and decision/failure metadata.
"""

from __future__ import annotations

from typing import Any

from mix_agent.db.models import Memory, MemoryEvidence
from mix_agent.memory import types as mem_types


def role_label(role: str | None) -> str:
    return {
        mem_types.ROLE_FACT: "事実",
        mem_types.ROLE_DECISION: "Decision",
        mem_types.ROLE_PREFERENCE: "Preference",
        mem_types.ROLE_GOAL: "Goal",
        mem_types.ROLE_CONSTRAINT: "Constraint",
        mem_types.ROLE_EXPERIENCE: "Experience",
        mem_types.ROLE_FAILURE: "Failure",
        mem_types.ROLE_SOLUTION: "Solution",
        mem_types.ROLE_OBSERVATION: "Observation",
        mem_types.ROLE_HYPOTHESIS: "Hypothesis",
        mem_types.ROLE_QUESTION: "Pending Question",
    }.get(role or "", role or "memory")


def scope_label(scope: str | None) -> str:
    return {
        mem_types.SCOPE_WORKING: "Working",
        mem_types.SCOPE_TASK: "Task",
        mem_types.SCOPE_PROJECT: "Project",
        mem_types.SCOPE_USER: "User",
        mem_types.SCOPE_WORLD: "World",
    }.get(scope or "", scope or "user")


def lifecycle_label(state: str | None) -> str:
    return {
        mem_types.LIFECYCLE_CANDIDATE: "候補",
        mem_types.LIFECYCLE_ACTIVE: "Active",
        mem_types.LIFECYCLE_SUPERSEDED: "Superseded",
        mem_types.LIFECYCLE_EXPIRED: "Expired",
        mem_types.LIFECYCLE_ARCHIVED: "Archived",
        mem_types.LIFECYCLE_DISPUTED: "Disputed",
        # legacy aliases
        "established": "Active",
        "latent": "候補",
        "deleted": "Expired",
    }.get(state or "", state or "")


def _normalize_lifecycle(state: str | None) -> str:
    if not state:
        return mem_types.LIFECYCLE_ACTIVE
    return mem_types.LEGACY_LIFECYCLE_MAP.get(state, state)


def _decision_block(row: Memory, data: dict) -> dict | None:
    md = data.get("decision_metadata") if isinstance(data, dict) else None
    if not isinstance(md, dict):
        return None
    return {
        "decision": md.get("decision", ""),
        "reason": md.get("reason", ""),
        "alternatives": md.get("alternatives") or [],
        "rejected_alternatives": md.get("rejected_alternatives") or [],
        "source": md.get("source", ""),
        "status": md.get("status", "active"),
        "timestamp": md.get("timestamp", ""),
    }


def _failure_block(row: Memory, data: dict) -> dict | None:
    md = data.get("failure_metadata") if isinstance(data, dict) else None
    if not isinstance(md, dict):
        return None
    return {
        "attempt": md.get("attempt", ""),
        "outcome": md.get("outcome", "failed"),
        "reason": md.get("reason", ""),
        "lesson": md.get("lesson", ""),
        "environment": md.get("environment") or {},
        "related_tool": md.get("related_tool", ""),
        "retry_suggested": bool(md.get("retry_suggested", False)),
    }


def _experience_block(row: Memory, data: dict) -> dict | None:
    md = data.get("experience_metadata") if isinstance(data, dict) else None
    if not isinstance(md, dict):
        return None
    return {
        "what": md.get("what", ""),
        "where": md.get("where", ""),
        "when": md.get("when", ""),
        "outcome": md.get("outcome", ""),
        "follow_up": md.get("follow_up", ""),
    }


def _evidence_summary(evidence_rows: list[MemoryEvidence]) -> list[dict]:
    summary: list[dict] = []
    for row in evidence_rows:
        data = row.data or {}
        summary.append({
            "id": row.id,
            "kind": row.kind,
            "ref": row.ref or data.get("ref", ""),
            "summary": row.summary or data.get("summary", ""),
            "captured_at": (row.captured_at.isoformat() if row.captured_at else None),
            "data": data,
        })
    return summary


def render(
    row: Memory,
    evidence_rows: list[MemoryEvidence] | None = None,
    *,
    trace: dict | None = None,
    score: float | None = None,
    selection_reason: str | None = None,
) -> dict:
    """Produce the canonical public view of a Memory row."""
    data = dict(row.data or {})
    lifecycle = _normalize_lifecycle(row.lifecycle_state)
    role = row.role or data.get("role") or mem_types.ROLE_FACT
    scope = row.scope or data.get("scope") or mem_types.SCOPE_USER
    view: dict[str, Any] = {
        "id": row.id,
        "content": data.get("content") or data.get("gist", ""),
        "gist": data.get("gist", ""),
        "role": role,
        "scope": scope,
        "source": data.get("source_run") or "",
        "source_kind": row.source_kind or data.get("source_kind", mem_types.SOURCE_AGENT),
        "lifecycle": lifecycle,
        "lifecycle_label": lifecycle_label(lifecycle),
        "verification": row.verification or data.get("verification", mem_types.VERIFICATION_UNVERIFIED),
        "confidence": round(float(row.confidence), 4),
        "salience": round(float(row.salience), 4),
        "strength": round(float(row.strength), 4),
        "activation_count": int(row.activation_count or 0),
        "last_activated_at": (row.last_activated_at.isoformat() if row.last_activated_at else None),
        "last_reinforced_at": (row.last_reinforced_at.isoformat() if row.last_reinforced_at else None),
        "task_id": row.task_id,
        "superseded_by_id": row.superseded_by_id,
        "disputed_by_id": row.disputed_by_id,
        "entities": data.get("entities") or [],
        "concepts": data.get("concepts") or [],
        "metadata": data.get("metadata") or {},
        "pinned": bool(data.get("pinned", False)),
        "created_at": row.created_at.isoformat() if row.created_at else None,
    }
    role_block = _decision_block(row, data) if role == mem_types.ROLE_DECISION else None
    if role_block is None and role == mem_types.ROLE_FAILURE:
        role_block = _failure_block(row, data)
    if role_block is None and role == mem_types.ROLE_EXPERIENCE:
        role_block = _experience_block(row, data)
    if role_block:
        view["role_metadata"] = role_block
    if evidence_rows is not None:
        view["evidence"] = _evidence_summary(evidence_rows)
        view["evidence_count"] = len(evidence_rows)
    else:
        view["evidence_count"] = int(data.get("evidence_count", 0))
    if trace is not None:
        view["trace"] = trace
    if score is not None:
        view["relevance"] = round(float(score), 4)
    if selection_reason:
        view["selection_reason"] = selection_reason
    return view


def render_compact(row: Memory) -> dict:
    """Cheap render for retrieval-time budgets: drop heavy nested fields."""
    data = dict(row.data or {})
    return {
        "id": row.id,
        "content": data.get("content") or data.get("gist", ""),
        "gist": data.get("gist", ""),
        "role": row.role or mem_types.ROLE_FACT,
        "scope": row.scope or mem_types.SCOPE_USER,
        "lifecycle": _normalize_lifecycle(row.lifecycle_state),
        "confidence": round(float(row.confidence), 4),
        "salience": round(float(row.salience), 4),
        "entities": data.get("entities") or [],
        "concepts": data.get("concepts") or [],
        "task_id": row.task_id,
    }


def role_group_key(role: str) -> str:
    """Bucket roles for retrieval ranking & UI grouping."""
    if role in mem_types.SEMANTIC_ROLES:
        return "semantic"
    if role in mem_types.EPISODIC_ROLES:
        return "episodic"
    if role in mem_types.TENTATIVE_ROLES:
        return "tentative"
    return "other"
