"""Privacy-minimal per-call token usage recording for cost reporting.

Only token counts, model identity, and resolved prices are stored.  Prompt and
completion text never reach this module.
"""

from __future__ import annotations

from datetime import timedelta

from sqlalchemy import delete

from mix_agent import pricing
from mix_agent.db.models import ModelUsageEvent, now

RETENTION = timedelta(days=90)

_INPUT_KEYS = ("input_tokens", "prompt_tokens", "promptTokenCount", "prompt_token_count")
_OUTPUT_KEYS = ("output_tokens", "completion_tokens", "candidatesTokenCount", "candidates_token_count")
_TOTAL_KEYS = ("total_tokens", "totalTokenCount", "total_token_count")
_CACHED_KEYS = ("cache_read_input_tokens", "cachedContentTokenCount", "cached_tokens", "cached_content_token_count")


def _count(source, keys):
    if not isinstance(source, dict):
        return None
    for key in keys:
        value = source.get(key)
        if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
            return value
    return None


def token_usage(usage):
    """Normalize the common provider usage shapes, or return ``None``."""
    if not isinstance(usage, dict) or not usage:
        return None
    input_tokens = _count(usage, _INPUT_KEYS)
    output_tokens = _count(usage, _OUTPUT_KEYS)
    total_tokens = _count(usage, _TOTAL_KEYS)
    if input_tokens is None and output_tokens is None and total_tokens is None:
        return None
    cached_tokens = _count(usage, _CACHED_KEYS)
    if cached_tokens is None:
        cached_tokens = _count(usage.get("prompt_tokens_details"), ("cached_tokens",))
    reasoning_tokens = _count(usage, ("reasoning_tokens", "thoughts_token_count"))
    if reasoning_tokens is None:
        reasoning_tokens = _count(usage.get("completion_tokens_details"), ("reasoning_tokens",))
    input_tokens = input_tokens or 0
    output_tokens = output_tokens or 0
    return {
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "cached_tokens": cached_tokens or 0,
        "reasoning_tokens": reasoning_tokens or 0,
        "total_tokens": total_tokens if total_tokens is not None else input_tokens + output_tokens,
    }


def record(db, owner_id, model_data, provider_id, mode, usage, *, run_id=None,
           input_estimate=None, model_record_id=None):
    """Store one model call's usage with its resolved price snapshot.

    Falls back to the locally estimated input token count only when the provider
    omitted usage entirely, and marks that row so the dashboard can separate
    measured from estimated usage.
    """
    current = now()
    db.execute(delete(ModelUsageEvent).where(
        ModelUsageEvent.owner_id == owner_id,
        ModelUsageEvent.created_at < current - RETENTION,
    ))
    tokens = token_usage(usage)
    source = "provider"
    if tokens is None or (not tokens["input_tokens"] and not tokens["output_tokens"]):
        if input_estimate:
            estimate = int(input_estimate)
            tokens = {
                "input_tokens": estimate, "output_tokens": 0, "cached_tokens": 0,
                "reasoning_tokens": 0, "total_tokens": estimate,
            }
            source = "estimated"
        else:
            return None
    prices, price_source = pricing.resolve(model_data or {})
    event = ModelUsageEvent(owner_id=owner_id, data={
        "model_id": (model_data or {}).get("model_id"),
        "model_record_id": model_record_id,
        "provider_id": provider_id,
        "mode": mode,
        "input_tokens": tokens["input_tokens"],
        "output_tokens": tokens["output_tokens"],
        "cached_tokens": tokens["cached_tokens"],
        "reasoning_tokens": tokens["reasoning_tokens"],
        "total_tokens": tokens["total_tokens"],
        "input_source": source,
        "pricing": prices,
        "pricing_source": price_source,
        "cost_usd": pricing.estimate(
            prices, tokens["input_tokens"], tokens["output_tokens"], tokens["cached_tokens"]
        ),
        "run_id": run_id,
    }, created_at=current)
    db.add(event)
    return event
