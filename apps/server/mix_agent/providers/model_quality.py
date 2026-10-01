"""Local, deterministic quality and cost priors for Auto model routing.

No provider calls happen here: quality comes from static capability tiers and
an identifier-derived parameter-size heuristic, while cost reuses the pricing
already resolved and stored on each model record (see ``mix_agent.pricing``).
"""

from __future__ import annotations

import re

from mix_agent import pricing

# Coarse capability tiers for well-known families.  The longest matching prefix
# wins, so a specific family always beats a generic one within the same kind.
_TIERS: dict[str, tuple[tuple[str, float], ...]] = {
    "openai": (
        ("gpt-4.1-mini", 0.55),
        ("gpt-4.1", 0.78),
        ("gpt-4o-mini", 0.45),
        ("gpt-4o", 0.70),
        ("o4-mini", 0.60),
        ("o3", 0.85),
        ("o1", 0.78),
        ("gpt-4", 0.55),
    ),
    "anthropic": (
        ("claude-opus", 0.90),
        ("claude-sonnet", 0.75),
        ("claude-haiku", 0.48),
        ("claude-3", 0.55),
    ),
    "gemini": (
        ("gemini-2.5-pro", 0.85),
        ("gemini-2.5-flash", 0.58),
        ("gemini-2", 0.62),
        ("gemini-1.5-pro", 0.58),
        ("gemini-1.5-flash", 0.42),
    ),
}

# Parameter count in billions, e.g. ``...-70b`` or ``...-1.5b``.
_SIZE = re.compile(r"(?:^|[^0-9.])(\d+(?:\.\d+)?)\s*b(?:[^a-z]|$)", re.IGNORECASE)
_SIZE_TIERS = (
    (200, 0.85),
    (100, 0.78),
    (60, 0.68),
    (30, 0.55),
    (12, 0.45),
    (6, 0.35),
    (2, 0.22),
)


def _identifier(model_data):
    return str(model_data.get("model_id") or "").casefold().removeprefix("models/")


def _tier(identifier):
    best = None
    for entries in _TIERS.values():
        for candidate in (identifier, identifier.rsplit("/", 1)[-1]):
            for prefix, value in entries:
                if candidate.startswith(prefix) and (best is None or len(prefix) > best[0]):
                    best = (len(prefix), value)
    return best[1] if best else None


def _size(identifier):
    match = _SIZE.search(identifier)
    if not match:
        return None
    try:
        parameters = float(match.group(1))
    except ValueError:
        return None
    for threshold, value in _SIZE_TIERS:
        if parameters >= threshold:
            return value
    return 0.15


def effective_flags(model_data):
    flags = {}
    flags.update(model_data.get("capabilities") or {})
    flags.update(model_data.get("overrides") or {})
    return flags


def quality_prior(model_data, provider_kind=None):
    """Return a neutral capability prior in ``[0, 1]`` or ``None`` if unknown."""
    identifier = _identifier(model_data)
    if not identifier:
        return None
    value = _tier(identifier)
    if value is None:
        value = _size(identifier)
    if value is None:
        return None
    # Size alone understates reasoning-tuned models, so nudge known reasoners up.
    if effective_flags(model_data).get("reasoning") is True:
        value = min(1.0, value + 0.04)
    return value


def request_cost(model_data, input_tokens, output_tokens):
    """Return an estimated USD cost for one request, or ``None`` when unknown."""
    prices, _ = pricing.resolve(model_data)
    if not prices:
        return None
    return pricing.estimate(prices, input_tokens=input_tokens, output_tokens=output_tokens)
