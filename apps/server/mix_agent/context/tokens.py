"""Token counting behind a single interface.

Exact tokenizers can be plugged in later without touching call sites.
Today everything funnels through :func:`count`, which uses a
model-specific approximation or a conservative generic fallback.
"""

from __future__ import annotations

import math


def _generic_estimate(text: str) -> int:
    if not text:
        return 0
    # UTF-8/2 is intentionally conservative for Japanese, code and mixed input.
    # Kept in exactly one place so budget math stays consistent.
    return math.ceil(len(text.encode("utf-8")) / 2)


def _model_adjustment(model_id: str, base: int) -> int:
    lowered = (model_id or "").lower()
    if any(key in lowered for key in ("claude", "sonnet", "opus")):
        # Anthropic-family tokenizers split CJK slightly finer than UTF-8/2.
        return math.ceil(base * 1.1)
    if "gemini" in lowered:
        return math.ceil(base * 0.95)
    return base


def count(text: str, model_id: str = "") -> int:
    """Best-effort input-token estimate for a single string."""
    return _model_adjustment(model_id or "", _generic_estimate(text or ""))


def count_messages(messages: list[dict], model_id: str = "") -> int:
    """Estimate tokens for provider messages, including tool/image overhead."""
    total = 0
    for message in messages or []:
        total += count(str(message.get("content") or ""), model_id)
        for image in message.get("images") or []:
            # Base64 images dominate context; approximate decoded bytes / 2.
            total += math.ceil(len(str(image)) / 3)
        for call in message.get("tool_calls") or []:
            total += count(str(call.get("arguments") or ""), model_id) + 40
        total += 8  # per-message framing overhead
    return total


def count_tool_schemas(tools: list[dict], model_id: str = "") -> int:
    """Estimate tokens consumed by tool definitions sent alongside messages."""
    import json

    total = 0
    for tool in tools or []:
        total += count(json.dumps(tool, ensure_ascii=False), model_id) + 20
    return total
