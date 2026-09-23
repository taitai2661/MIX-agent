"""Model-aware context budgets. Replaces the fixed 180000-char limit."""

from __future__ import annotations

from mix_agent.context import tokens
from mix_agent.context.types import DEFAULT_SHARES

# Safe-side fallback for models with unknown windows.
FALLBACK_CONTEXT_WINDOW = 32_000
FALLBACK_RESERVED_OUTPUT = 4096
FALLBACK_SAFETY_MARGIN = 2000


def resolve_window(model_data: dict | None, snapshot: dict | None = None) -> dict:
    """Return {context_window, reserved_output_tokens, safety_margin} for a model."""
    data = dict(model_data or {})
    if snapshot:
        for key in ("context_window",):
            if snapshot.get(key):
                data.setdefault(key, snapshot.get(key))
    window = data.get("context_window")
    if not isinstance(window, int) or isinstance(window, bool) or window <= 0:
        window = FALLBACK_CONTEXT_WINDOW
    reserved = None
    for source in (snapshot or {}, data):
        candidate = (source.get("model_settings") or {}).get("max_output_tokens")
        if isinstance(candidate, int) and not isinstance(candidate, bool) and candidate > 0:
            reserved = candidate
            break
    if reserved is None:
        candidate = data.get("max_output_tokens")
        reserved = (candidate if isinstance(candidate, int) and not isinstance(candidate, bool) and candidate > 0
                    else FALLBACK_RESERVED_OUTPUT)
    # Safety margin scales mildly with window size so small models stay usable.
    safety = min(8000, max(1000, window // 32))
    if (isinstance(data.get("safety_margin"), int) and not isinstance(data["safety_margin"], bool)
            and data["safety_margin"] >= 0):
        safety = data["safety_margin"]
    return {
        "context_window": window,
        "reserved_output_tokens": reserved,
        "safety_margin": safety,
    }


def input_budget(window_info: dict, tool_schema_tokens: int = 0) -> int:
    """Tokens available for input after reserving output, safety and tool schemas."""
    return max(
        4000,
        int(window_info["context_window"])
        - int(window_info["reserved_output_tokens"])
        - int(window_info["safety_margin"])
        - int(tool_schema_tokens or 0),
    )


def category_budgets(total: int, shares: dict | None = None) -> dict[str, int]:
    """Split an input budget into per-category token budgets."""
    active = {**DEFAULT_SHARES, **(shares or {})}
    return {key: max(200, int(total * active.get(key, 0))) for key in active if key != "reserve"}


def reflow(unused: dict[str, int], budgets: dict[str, int]) -> dict[str, int]:
    """Return leftover tokens from unused categories to recent_conversation."""
    spare = sum(max(0, unused.get(key, 0)) for key in unused)
    result = dict(budgets)
    result["recent_conversation"] = result.get("recent_conversation", 0) + spare
    return result


def estimate_for_routing(parts: list[str], attachment_bytes: int = 0, model_id: str = "") -> int:
    """Single funnel for routing estimates (replaces scattered utf8/2 math)."""
    total = sum(tokens.count(part or "", model_id) for part in parts)
    total += (attachment_bytes or 0) // 2
    return total
