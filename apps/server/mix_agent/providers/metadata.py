"""Evidence-backed model metadata resolution, independent from wire transports."""

from __future__ import annotations

from datetime import UTC, datetime

from mix_agent.providers import model_info

SENSITIVE_KEYS = {"api_key", "key", "token", "authorization", "owner", "organization", "org_id"}


def redact(value):
    if isinstance(value, dict):
        return {key: redact(item) for key, item in value.items() if key.lower() not in SENSITIVE_KEYS}
    if isinstance(value, list):
        return [redact(item) for item in value]
    return value


def context_limit(item):
    for key in ("context_length", "context_window", "max_context_length", "max_context_tokens", "inputTokenLimit"):
        value = item.get(key)
        if isinstance(value, int) and not isinstance(value, bool) and value > 0:
            return value
        if isinstance(value, str) and value.isdecimal() and int(value) > 0:
            return int(value)
    # Catalogs and gateways nest the window as ``limit.context`` or
    # ``limits.context``.  Both spellings are documented wire shapes.
    for key in ("limits", "limit"):
        limits = item.get(key)
        if (isinstance(limits, dict) and isinstance(limits.get("context"), int)
                and not isinstance(limits["context"], bool) and limits["context"] > 0):
            return limits["context"]
    return None


def _record(values, source, confidence):
    stamp = datetime.now(UTC).isoformat()
    return {key: {"value": value, "source": source, "confidence": confidence, "resolved_at": stamp}
            for key, value in values.items() if value is not None}


async def resolve(kind, model_id, provider_item, resolver_ids=("provider_api", "model_info"),
                  *, preset_id=None):
    """Return normalized effective metadata and all non-manual candidates."""
    candidates = {}
    api_values = {"context_window": context_limit(provider_item),
                  "max_output_tokens": provider_item.get("max_output_tokens") or provider_item.get("outputTokenLimit")}
    if "provider_api" in resolver_ids and any(value is not None for value in api_values.values()):
        candidates["provider_api"] = _record(api_values, "provider_api", "official")
    external = {}
    if "model_info" in resolver_ids:
        external = await model_info.lookup(model_info.provider_keys(preset_id), model_id)
    if any(value is not None for value in external.values()):
        candidates["model_info"] = _record(external, "model_info", "external")
    effective = {}
    for source in ("provider_api", "model_info"):
        for field, evidence in candidates.get(source, {}).items():
            effective.setdefault(field, evidence)
    return {"provider_metadata": redact(provider_item), "metadata_candidates": candidates, "metadata": effective}
