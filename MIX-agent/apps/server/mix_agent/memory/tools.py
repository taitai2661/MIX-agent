"""ChatGPT-style Memory tools: recall / remember / forget (flat)."""

from __future__ import annotations

from typing import Any

from mix_agent.memory import runtime as mem_runtime
from mix_agent.memory import views as mem_views


def _safe_text(value: Any, limit: int = 2000) -> str:
    text = str(value or "")
    if len(text) > limit:
        return text[:limit] + "…"
    return text


def recall(db, owner: str, *, query: str = "", task: dict | None = None, limit: int = 8, **kwargs) -> dict:
    if not task and query:
        task = {"text": query, "kind": "tool_recall"}
    elif not task:
        task = {"text": "", "kind": "tool_recall"}
    memories = mem_runtime.recall(db, owner, task, limit=max(1, min(20, int(limit or 8))))
    return {"memories": [mem_views.render_compact(m) for m in memories], "grouped": {"all": [mem_views.render_compact(m) for m in memories]}, "total": len(memories)}


def remember(db, owner: str, *, content: str, explicit_user: bool = False, **kwargs) -> dict:
    if not content or not content.strip():
        raise ValueError("memory content is empty")
    evaluation = mem_runtime.evaluate(db, owner, observation={"summary": _safe_text(content, 600), "content": _safe_text(content, 4000)}, explicit_user=explicit_user)
    applied = mem_runtime.apply_plan(db, owner, evaluation.get("plan") or [])
    db.flush()
    return {"decision": evaluation.get("reason"), "applied": applied}


def forget(db, owner: str, *, memory_id: str | None = None, query: str | None = None, reason: str = "explicit forget", **kwargs) -> dict:
    target_id = memory_id or kwargs.get("id")
    if not target_id:
        if not query:
            raise ValueError("memory_id or query required")
        matches = mem_runtime.recall(db, owner, {"text": query, "kind": "tool_forget"}, limit=5)
        if not matches:
            return {"forgotten": [], "reason": "no match"}
        target_id = matches[0]["id"]
    from mix_agent.db.models import Memory as _Memory

    row = db.get(_Memory, target_id)
    if not row or row.owner_id != owner:
        raise ValueError("Memory not found")
    mem_runtime.supersede(db, owner, old_id=target_id, new_id=None, reason=reason[:200])
    db.flush()
    return {"forgotten": [target_id], "reason": reason}
