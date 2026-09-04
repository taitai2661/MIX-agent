"""Evidence-backed model metadata resolution, independent from wire transports."""

from __future__ import annotations

from datetime import datetime, timezone
import httpx


# Exact records only: never derive a limit from an identifier prefix.
OFFICIAL_CATALOG = {
    ("openai", "gpt-4o"): {"context_window": 128_000, "max_output_tokens": 16_384},
    ("openai", "gpt-4o-mini"): {"context_window": 128_000, "max_output_tokens": 16_384},
    ("openai", "gpt-4.1"): {"context_window": 1_047_576, "max_output_tokens": 32_768},
    ("anthropic", "claude-sonnet-4-5"): {"context_window": 200_000},
    ("anthropic", "claude-sonnet-4-6"): {"context_window": 200_000},
    ("gemini", "gemini-2.5-flash"): {"context_window": 1_048_576},
}
BUILTIN_CATALOG = {
    ("openai", "gpt-4-turbo"): {"context_window": 128_000},
    ("gemini", "gemini-1.5-pro"): {"context_window": 1_048_576},
}
SENSITIVE_KEYS = {"api_key", "key", "token", "authorization", "owner", "organization", "org_id"}

# Safe-side fallback for models with unknown windows (see context/budget.py).
FALLBACK_CONTEXT_WINDOW = 32_000
FALLBACK_RESERVED_OUTPUT_TOKENS = 4096
FALLBACK_SAFETY_MARGIN_TOKENS = 2000


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
    limits = item.get("limits")
    if (isinstance(limits, dict) and isinstance(limits.get("context"), int)
            and not isinstance(limits["context"], bool) and limits["context"] > 0):
        return limits["context"]
    return None


def _record(values, source, confidence):
    stamp = datetime.now(timezone.utc).isoformat()
    return {key: {"value": value, "source": source, "confidence": confidence, "resolved_at": stamp}
            for key, value in values.items() if value is not None}


async def models_dev(kind, model_id):
    """Fetch models.dev opportunistically; failure must never block a provider sync."""
    try:
        async with httpx.AsyncClient(timeout=5, follow_redirects=False) as client:
            response = await client.get("https://models.dev/api.json")
            response.raise_for_status()
            catalog = response.json()
    except Exception:
        return {}
    providers = catalog.get("providers", catalog) if isinstance(catalog, dict) else {}
    candidate = providers.get(kind, {}) if isinstance(providers, dict) else {}
    models = candidate.get("models", candidate) if isinstance(candidate, dict) else {}
    row = models.get(model_id, {}) if isinstance(models, dict) else {}
    if not isinstance(row, dict):
        return {}
    return {"context_window": context_limit(row), "max_output_tokens": row.get("max_output_tokens") or row.get("output"),
            "modalities": row.get("modalities"), "pricing": row.get("cost") or row.get("pricing")}


async def resolve(kind, model_id, provider_item, resolver_ids=("provider_api", "official_catalog", "models_dev", "builtin")):
    """Return normalized effective metadata and all non-manual candidates."""
    candidates = {}
    api_values = {"context_window": context_limit(provider_item),
                  "max_output_tokens": provider_item.get("max_output_tokens") or provider_item.get("outputTokenLimit")}
    if "provider_api" in resolver_ids and any(value is not None for value in api_values.values()):
        candidates["provider_api"] = _record(api_values, "provider_api", "official")
    official = OFFICIAL_CATALOG.get((kind, model_id), {})
    if "official_catalog" in resolver_ids and official:
        candidates["official_catalog"] = _record(official, "official_catalog", "official")
    external = await models_dev(kind, model_id) if "models_dev" in resolver_ids else {}
    if any(value is not None for value in external.values()):
        candidates["models_dev"] = _record(external, "models_dev", "external")
    builtin = BUILTIN_CATALOG.get((kind, model_id), {})
    if "builtin" in resolver_ids and builtin:
        candidates["builtin"] = _record(builtin, "builtin", "fallback")
    effective = {}
    for source in ("provider_api", "official_catalog", "models_dev", "builtin"):
        for field, evidence in candidates.get(source, {}).items():
            effective.setdefault(field, evidence)
    return {"provider_metadata": redact(provider_item), "metadata_candidates": candidates, "metadata": effective}
