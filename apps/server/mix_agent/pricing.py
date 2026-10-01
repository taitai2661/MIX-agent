"""Local model price resolution for usage cost estimates.

All prices are USD per 1,000,000 tokens.  This module never contacts a
provider: the model-info catalog is resolved during a model sync and stored on
the model record, while an administrator can override it per model.
"""

from __future__ import annotations

import math

MILLION = 1_000_000

_ALIASES = {
    "input": ("input", "prompt", "input_cost_per_token", "prompt_cost_per_token"),
    "output": ("output", "completion", "output_cost_per_token", "completion_cost_per_token"),
    "cache_read": ("cache_read", "cache_read_input_tokens", "cached", "cached_input"),
    "cache_write": ("cache_write", "cache_creation_input_tokens", "cached_write"),
}
# model-info states a price unit explicitly; anything else (per-request fees,
# unknown units) is unusable for token-based estimates and must not be guessed.
_UNITS = {"1M_tokens": 1.0, "1K_tokens": 1_000.0, "1_token": 1_000_000.0}


def _number(value):
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)) and math.isfinite(value) and value >= 0:
        return float(value)
    return None


def normalize(raw):
    """Return USD-per-million prices from a catalog row, or ``None``."""
    if not isinstance(raw, dict):
        return None
    currency = raw.get("currency")
    if currency is not None and currency != "USD":
        return None
    unit = raw.get("unit")
    factor = 1.0 if unit is None else _UNITS.get(unit)
    if factor is None:
        return None
    prices = {}
    for field, keys in _ALIASES.items():
        for key in keys:
            value = _number(raw.get(key))
            if value is not None:
                prices[field] = value * factor
                break
    if prices.get("input") is None and prices.get("output") is None:
        return None
    return {
        "input": prices.get("input"),
        "output": prices.get("output"),
        "cache_read": prices.get("cache_read"),
        "cache_write": prices.get("cache_write"),
    }


def resolve(model_data):
    """Return ``(prices, source)`` with a manual override taking precedence."""
    model_data = model_data or {}
    override = normalize(model_data.get("pricing_override"))
    if override:
        return override, "manual"
    metadata = model_data.get("metadata")
    evidence = metadata.get("pricing") if isinstance(metadata, dict) else None
    value = evidence.get("value") if isinstance(evidence, dict) else evidence
    external = normalize(value)
    if external:
        source = evidence.get("source") if isinstance(evidence, dict) else None
        return external, source or "model_info"
    return None, "unknown"


def estimate(prices, input_tokens=0, output_tokens=0, cached_tokens=0):
    """Return an estimated USD cost, or ``None`` when prices are unusable."""
    if not prices:
        return None
    input_tokens = max(0, int(input_tokens or 0))
    output_tokens = max(0, int(output_tokens or 0))
    cached_tokens = min(input_tokens, max(0, int(cached_tokens or 0)))
    total = 0.0
    for tokens, field in (
        (input_tokens - cached_tokens, "input"),
        (output_tokens, "output"),
        (cached_tokens, "cache_read"),
    ):
        if tokens <= 0:
            continue
        price = prices.get(field)
        # Without a dedicated cache rate, cached input is priced as input.
        if price is None and field == "cache_read":
            price = prices.get("input")
        if price is None:
            return None
        total += tokens / MILLION * price
    return round(total, 8)
