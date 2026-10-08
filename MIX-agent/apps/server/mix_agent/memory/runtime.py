"""ChatGPT-style Memory Runtime: flat recall + simple evaluator.

- ``recall``: text-match + recency over flat Memory rows.
- ``evaluate``/``apply_plan``: duplicate -> reinforce (noop), else create.
  No supersede/dispute/conflict graph. ``supersede``/``dispute`` remain as
  compatibility shims mapping to update/delete.
- ``group_for_context``: single ``all`` bucket (+ legacy bucket keys as
  empty lists so old readers don't crash).
"""

from __future__ import annotations

import logging
from collections.abc import Iterable
from datetime import UTC, datetime

from sqlalchemy import func, select

from mix_agent.db.models import Memory, MemoryActionEvent, MemoryRevision, now
from mix_agent.memory import service as legacy_service
from mix_agent.memory import views as mem_views

LOGGER = logging.getLogger(__name__)

RECALL_DEFAULTS: dict[str, float] = {"result_limit": 8, "lexical_weight": 0.85, "recency_weight": 0.15}


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


def recall(db, owner: str, task, *, scopes=None, roles=None, include_lifecycles=None, limit=None, settings=None, debug=False, **kwargs):
    query = _query_text(task)
    n = max(1, min(50, int(limit or (settings or {}).get("result_limit", 8) if isinstance(settings, dict) else 8)))
    found = legacy_service.search(db, owner, query, settings={"result_limit": n}, debug=False)
    views = []
    for item in found:
        row = db.get(Memory, item["id"])
        if not row:
            continue
        views.append(mem_views.render(row, score=item.get("relevance"), selection_reason=item.get("selection_reason")))
    if debug:
        return {"memories": views, "debug": {"considered": len(views), "kept": len(views), "weights": {"lexical": 0.85, "recency": 0.15}, "lexical": 0.85, "task_keywords": sorted(legacy_service.terms(query))[:20], "scopes": [], "roles": [], "lifecycles": ["active"], "elapsed_ms": 0.0, "budget_exhausted": False}}
    return views


def evaluate(db, owner: str, *, task_id=None, observation=None, tool_call=None, plan_step=None, explicit_user=False, **kwargs) -> dict:
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
    for row in db.scalars(select(Memory).where(Memory.owner_id == owner).order_by(Memory.updated_at.desc()).limit(200)):
        existing = legacy_service._content(row)
        if legacy_service._normalize(existing) == normalized:
            return {"plan": [{"action": "reinforce", "memory_id": row.id, "reason": "duplicate"}], "skipped": False, "reason": "duplicate"}
    candidate = {"content": content, "gist": summary, "explicit_user": explicit_user}
    return {"plan": [{"action": "create", "candidate": candidate}], "skipped": False, "reason": "new"}


def apply_plan(db, owner: str, plan: list[dict], *, run_id: str | None = None) -> list[dict]:
    results: list[dict] = []
    for step in plan or []:
        action = step.get("action")
        if action == "create":
            candidate = step.get("candidate") or {}
            content = str(candidate.get("content") or candidate.get("gist") or "").strip()
            if not content:
                continue
            created = legacy_service.change(db, owner, content, source_run=run_id, pinned=False)
            row = db.get(Memory, created["id"])
            results.append({"action": "create", "id": row.id, "memory": mem_views.render(row)})
        elif action == "reinforce":
            row = db.get(Memory, step.get("memory_id"))
            if row and row.owner_id == owner:
                row.updated_at = now()
                results.append({"action": "reinforce", "id": row.id})
        elif action == "no_op":
            continue
        elif action in ("supersede", "update"):
            # compat: treat as content update
            target = step.get("memory_id")
            candidate = step.get("candidate") or {}
            content = str(candidate.get("content") or "").strip()
            if target and content:
                try:
                    legacy_service.change(db, owner, content, memory_id=target, source_run=run_id)
                    results.append({"action": "update", "id": target})
                except ValueError:
                    continue
        elif action == "dispute":
            candidate = step.get("candidate") or {}
            content = str(candidate.get("content") or "").strip()
            if content:
                created = legacy_service.change(db, owner, content, source_run=run_id)
                results.append({"action": "create", "id": created["id"]})
        else:
            LOGGER.warning("Unknown memory plan action: %s", action)
        if run_id and action:
            _emit_event(db, owner, run_id, str(action).upper(), None, None, None, str(step.get("reason", "")))
    return results


def supersede(db, owner: str, *, old_id: str, new_id: str | None = None, reason: str = "") -> dict:
    """Compat shim: ChatGPT forget = hard delete (keep revision backup)."""
    old = db.get(Memory, old_id)
    if not old or old.owner_id != owner:
        raise ValueError("Old memory not found")
    db.add(MemoryRevision(owner_id=owner, data={"memory_id": old.id, "previous": {"id": old.id, "content": legacy_service._content(old)}}))
    db.delete(old)
    db.flush()
    _emit_event(db, owner, None, "SUPERSEDE", old_id, None, None, reason)
    return {"id": old_id, "superseded_by_id": new_id, "lifecycle": "superseded"}


def dispute(db, owner: str, *, memory_id: str, conflicting_id: str, reason: str = "") -> dict:
    _emit_event(db, owner, None, "DISPUTE", memory_id, None, None, reason)
    return {"a": memory_id, "b": conflicting_id, "lifecycle": "disputed"}


def resolve_conflict(db, owner: str, *, conflict_id: str, resolution: str, prefer: str | None = None) -> dict:
    return {"id": conflict_id, "resolution": resolution, "winner": prefer, "loser": None}


def attach_evidence(db, owner: str, memory_id: str, *, kind: str = "observation", ref: str = "", summary: str = "", data=None, captured_at=None, confidence=None) -> dict:
    row = db.get(Memory, memory_id)
    if not row or row.owner_id != owner:
        raise ValueError("Memory not found")
    row.updated_at = now()
    return {"id": memory_id, "memory_id": memory_id, "kind": kind}


def group_for_context(memories: list[dict]) -> dict[str, list[dict]]:
    grouped: dict[str, list[dict]] = {"all": list(memories or []), "decisions": [], "failures": [], "facts": list(memories or []), "preferences": [], "goals": [], "questions": [], "experiences": [], "other": []}
    return grouped


def compact_for_context(memories: list[dict], token_budget: int = 1500) -> list[dict]:
    if not memories:
        return []
    keep: list[dict] = []
    used = 0
    for mem in memories:
        compact = {"id": mem.get("id"), "content": str(mem.get("content", ""))[:600]}
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
    stored = db.scalar(select(func.count()).select_from(MemoryActionEvent).where(MemoryActionEvent.run_id == run_id))
    pending = sum(1 for obj in db.new if isinstance(obj, MemoryActionEvent) and obj.run_id == run_id)
    return int(stored or 0) + pending


def _emit_event(db, owner: str, run_id: str | None, action: str, memory_id: str | None, before, after, reason: str) -> None:
    db.add(MemoryActionEvent(owner_id=owner, run_id=run_id, candidate_index=_next_candidate_index(db, run_id), data={"action": action, "memory_id": memory_id, "reason": (reason or "")[:400], "runtime": "chatgpt_flat"}))
