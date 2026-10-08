"""ChatGPT-style public view: id / content / timestamps only."""

from __future__ import annotations

from typing import Any

from mix_agent.db.models import Memory


def _content(row: Memory) -> str:
    if getattr(row, "content", None):
        return row.content
    data = row.data or {}
    return data.get("content") or data.get("gist", "")


def render(row: Memory, evidence_rows=None, *, trace=None, score=None, selection_reason=None) -> dict:
    data = dict(row.data or {})
    view: dict[str, Any] = {
        "id": row.id,
        "content": _content(row),
        "gist": _content(row)[:300],
        "pinned": bool(data.get("pinned", False)),
        "source": data.get("source", "user"),
        "created_at": row.created_at.isoformat() if row.created_at else None,
        "updated_at": row.updated_at.isoformat() if getattr(row, "updated_at", None) and row.updated_at else None,
    }
    if score is not None:
        view["relevance"] = round(float(score), 4)
    if selection_reason:
        view["selection_reason"] = selection_reason
    # legacy keys for old readers (constants now)
    view.update({"role": "fact", "scope": "user", "lifecycle": "active"})
    return view


def render_compact(row: Memory) -> dict:
    # Accepts either a Memory row or an already-rendered dict.
    if isinstance(row, dict):
        content = row.get("content", "") or row.get("gist", "")
        return {"id": row.get("id"), "content": content, "gist": content[:300]}
    return {"id": row.id, "content": _content(row), "gist": _content(row)[:300]}


# Deprecated taxonomy label helpers kept so old imports don't crash.
def role_label(role=None) -> str:
    return "記憶"


def scope_label(scope=None) -> str:
    return "User"


def lifecycle_label(state=None) -> str:
    return "Active"


def role_group_key(role=None) -> str:
    return "all"
