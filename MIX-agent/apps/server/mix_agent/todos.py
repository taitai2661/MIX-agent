"""Conversation-scoped todo list derived from the agent plan.

The agent maintains a single checklist through ``update_plan``; its structured
result (``steps`` + ``pending``) is mirrored onto ``Conversation.data["todos"]``
so the user-visible list survives across runs of the same conversation.
"""

from __future__ import annotations

MAX_ITEMS = 30
MAX_CONTENT = 300
STATUSES = ("pending", "in_progress", "completed")


def _clean(values, limit=MAX_ITEMS) -> list[str]:
    cleaned = []
    for value in values or []:
        if not isinstance(value, str):
            continue
        text = value.strip()
        if not text or text in cleaned:
            continue
        cleaned.append(text[:MAX_CONTENT])
        if len(cleaned) >= limit:
            break
    return cleaned


def derive(agent_plan: dict | None) -> list[dict]:
    """Turn an ``update_plan`` result into ordered todo items.

    Items outside ``pending`` are completed; the first pending item is
    ``in_progress`` (Codex-style: one item is being worked on) and the rest are
    ``pending``. Pending entries that do not match a step keep their place as
    pending items instead of being dropped.
    """
    plan = agent_plan if isinstance(agent_plan, dict) else {}
    steps = _clean(plan.get("steps"))
    pending = _clean(plan.get("pending"))
    pending_set = set(pending)
    items = []
    started = False
    for step in steps:
        if step in pending_set:
            status = "in_progress" if not started else "pending"
            started = True
        else:
            status = "completed"
        items.append({"content": step, "status": status})
    known = set(steps)
    for extra in pending:
        if extra not in known:
            items.append({"content": extra, "status": "pending"})
    return items[:MAX_ITEMS]


def sanitize(raw) -> list[dict]:
    """Return a stored todo list trimmed to the display contract."""
    if not isinstance(raw, list):
        return []
    items = []
    for entry in raw[:MAX_ITEMS]:
        if not isinstance(entry, dict):
            continue
        content = entry.get("content")
        if not isinstance(content, str):
            continue
        content = content.strip()
        status = entry.get("status")
        if not content or status not in STATUSES:
            continue
        items.append({"content": content[:MAX_CONTENT], "status": status})
    return items


def render_for_prompt(raw) -> str:
    """Compact carry-over block for the next run's system context."""
    items = sanitize(raw)
    if not items:
        return ""
    marks = {"completed": "x", "in_progress": "~", "pending": " "}
    lines = [
        "- [" + marks[item["status"]] + "] " + item["content"]
        for item in items
    ]
    return (
        "Todo list carried over from earlier in this conversation. Keep "
        "update_plan.steps aligned with these items, keep unfinished ones in "
        "update_plan.pending, and clear the list with empty steps when no todo "
        "remains:\n" + "\n".join(lines)
    )
