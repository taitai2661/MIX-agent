"""Structured task state: validated JSON kept separately from free-form summary."""

from __future__ import annotations

from mix_agent.context.types import EMPTY_TASK_STATE, TASK_STATE_LIST_FIELDS


def blank() -> dict:
    return {key: ([] if isinstance(value, list) else "") for key, value in EMPTY_TASK_STATE.items()}


def _clean_list(values, limit=30, item_limit=300) -> list:
    cleaned = []
    for value in values or []:
        text = str(value or "").strip()
        if not text:
            continue
        cleaned.append(text[:item_limit])
        if len(cleaned) >= limit:
            break
    return cleaned


def validate(raw: dict | None) -> dict:
    """Validate LLM-produced state; fall back to blank without raising."""
    if not isinstance(raw, dict):
        return blank()
    state = blank()
    goal = str(raw.get("goal") or "").strip()
    state["goal"] = goal[:1000]
    for key in TASK_STATE_LIST_FIELDS:
        state[key] = _clean_list(raw.get(key))
    return state


def ensure(raw: dict | None, goal_fallback: str = "") -> dict:
    """Return existing valid state, or a blank one seeded with the user goal."""
    state = validate(raw)
    if not state["goal"] and goal_fallback:
        state["goal"] = str(goal_fallback)[:1000]
    return state


def merge(old: dict | None, update: dict | None) -> dict:
    """Merge an LLM checkpoint update; broken output keeps the old state."""
    base = validate(old)
    if not isinstance(update, dict) or not update:
        return base
    merged = validate({**base, **{k: update.get(k, base[k]) for k in base}})
    # Never drop a goal/pending silently: keep old values when update is empty.
    if not merged["goal"]:
        merged["goal"] = base["goal"]
    if not merged["pending"] and base["pending"]:
        merged["pending"] = base["pending"]
    return merged


def render(state: dict | None) -> str:
    """Compact text form injected into the system-adjacent context block."""
    state = validate(state)
    if not any(state.values()):
        return ""
    lines = []
    if state["goal"]:
        lines.append("Goal: " + state["goal"])
    for key in (
        "constraints",
        "plan",
        "completed",
        "pending",
        "important_facts",
        "artifacts",
        "open_questions",
    ):
        values = state.get(key) or []
        if values:
            lines.append(key + ": " + " | ".join(values[:12]))
    return "Task state (data):\n" + "\n".join(lines)
