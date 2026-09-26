"""Memory Tool abstractions.

The user-facing Memory tool surface is intentionally tiny: ``recall``,
``remember`` and ``forget``.  These wrap the role-based Memory Runtime so the
agent never touches raw CRUD.  The legacy ``memory_search`` / ``memory_add``
/ ``memory_update`` / ``memory_delete`` tools remain available as thin
compatibility shims that map onto the new operations; they call the same
runtime functions and emit the same audit events.
"""

from __future__ import annotations

import json
import logging
from typing import Any

from mix_agent.memory import runtime as mem_runtime
from mix_agent.memory import types as mem_types
from mix_agent.memory import views as mem_views

LOGGER = logging.getLogger(__name__)


def _safe_text(value: Any, limit: int = 2000) -> str:
    text = str(value or "")
    if len(text) > limit:
        return text[:limit] + "…"
    return text


def recall(db, owner: str, *, query: str = "", task: dict | None = None, scopes: list[str] | None = None,
           roles: list[str] | None = None, limit: int = 8) -> dict:
    """Recall the memories relevant to the current task.

    ``task`` may be either a plain string or a structured task description
    (goal / keywords / pending failures).  The Recall Pipeline combines
    semantic similarity, role-aware boosts and confidence gating so the agent
    sees only memories that actually help.
    """
    if not task and query:
        task = {"text": query, "kind": "tool_recall"}
    elif not task:
        task = {"text": "", "kind": "tool_recall"}
    memories = mem_runtime.recall(
        db,
        owner,
        task,
        scopes=scopes,
        roles=roles,
        limit=max(1, min(20, int(limit or 8))),
    )
    grouped = mem_runtime.group_for_context(memories)
    return {
        "memories": [mem_views.render_compact(_to_row_shape(item)) for item in memories],
        "grouped": {key: [mem_views.render_compact(_to_row_shape(item)) for item in items] for key, items in grouped.items()},
        "total": len(memories),
    }


def remember(
    db,
    owner: str,
    *,
    content: str,
    role: str = mem_types.ROLE_FACT,
    scope: str = mem_types.SCOPE_USER,
    role_metadata: dict | None = None,
    entities: list[str] | None = None,
    concepts: list[str] | None = None,
    task_id: str | None = None,
    source_kind: str | None = None,
    verification: str | None = None,
    evidence: list[dict] | None = None,
    explicit_user: bool = False,
) -> dict:
    """Persist a memory the agent just learned.

    Routes through the Memory Evaluator so duplicates are reinforced,
    contradictions open a dispute, and lifecycle decisions are made
    consistently with the rest of the runtime.
    """
    if not content or not content.strip():
        raise ValueError("memory content is empty")
    evaluation = mem_runtime.evaluate(
        db,
        owner,
        task_id=task_id,
        observation={
            "summary": _safe_text(content, 600),
            "content": _safe_text(content, 4000),
            "role": role,
            "scope": scope,
            "source_kind": source_kind or (mem_types.SOURCE_USER if explicit_user else mem_types.SOURCE_AGENT),
            "role_metadata": role_metadata,
            "entities": entities,
            "concepts": concepts,
            "verification": verification,
            "evidence": evidence,
        },
        explicit_user=explicit_user,
    )
    applied = mem_runtime.apply_plan(db, owner, evaluation.get("plan") or [])
    db.flush()
    return {
        "decision": evaluation.get("reason"),
        "applied": applied,
    }


def forget(
    db,
    owner: str,
    *,
    memory_id: str | None = None,
    query: str | None = None,
    reason: str = "explicit forget",
) -> dict:
    """Remove (or supersede) a Memory item.

    If ``memory_id`` is given, the item is marked ``superseded`` (lifecycle)
    so the agent can later recall *why* it was forgotten.  Otherwise the
    function runs the recall pipeline to find the best-matching item.
    """
    target_id = memory_id
    if not target_id:
        if not query:
            raise ValueError("memory_id or query required")
        matches = mem_runtime.recall(db, owner, {"text": query, "kind": "tool_forget"}, limit=5)
        if not matches:
            return {"forgotten": [], "reason": "no match"}
        # Surface only the top match to keep forget semantics predictable.
        target_id = matches[0]["id"]
    row = db.get(__import__("mix_agent.db.models", fromlist=["Memory"]).Memory, target_id)
    if not row or row.owner_id != owner:
        raise ValueError("Memory not found")
    mem_runtime.supersede(db, owner, old_id=target_id, new_id=None, reason=reason[:200])
    db.flush()
    return {"forgotten": [target_id], "reason": reason}


def _to_row_shape(item: dict) -> dict:
    """Convert a Memory Runtime view into a shape the legacy tools expect."""
    return {
        "id": item.get("id"),
        "data": {
            "content": item.get("content", ""),
            "gist": item.get("gist", ""),
            "scope": item.get("scope"),
            "deleted": item.get("lifecycle") == mem_types.LIFECYCLE_EXPIRED,
            "pinned": item.get("pinned", False),
            "entities": item.get("entities", []),
            "concepts": item.get("concepts", []),
        },
        "lifecycle_state": item.get("lifecycle"),
        "role": item.get("role"),
        "verification": item.get("verification"),
        "confidence": item.get("confidence"),
        "salience": item.get("salience"),
        "strength": item.get("strength"),
        "selection_reason": item.get("selection_reason"),
        "relevance": item.get("relevance"),
    }
