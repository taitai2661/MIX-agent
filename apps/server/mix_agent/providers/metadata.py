"""Evidence-backed model metadata resolution, independent from wire transports."""

from __future__ import annotations

import time
from datetime import UTC, datetime

import httpx

# Exact records only: never derive a limit from an identifier prefix.
OFFICIAL_CATALOG = {
    ("openai", "gpt-4o"): {"context_window": 128_000, "max_output_tokens": 16_384},
    ("openai", "gpt-4o-mini"): {"context_window": 128_000, "max_output_tokens": 16_384},
    ("openai", "gpt-4.1"): {"context_window": 1_047_576, "max_output_tokens": 32_768},
    ("anthropic", "claude-sonnet-4-5"): {"context_window": 200_000},
    ("anthropic", "claude-sonnet-4-6"): {"context_window": 200_000},
    ("gemini", "gemini-2.5-flash"): {"context_window": 1_048_576},
    # Upstream models hosted on NVIDIA NIM and other compatible gateways.
    # Architecture-level windows, safe regardless of which gateway serves them.
    ("compatible", "meta/llama-3.3-70b-instruct"): {"context_window": 128_000},
    ("compatible", "meta/llama-3.1-70b-instruct"): {"context_window": 128_000},
    ("compatible", "mistralai/mistral-large-3-675b-instruct-2512"): {"context_window": 262_144},
    ("compatible", "deepseek-ai/deepseek-v4-pro"): {"context_window": 1_048_576},
    ("compatible", "qwen/qwen3-coder-480b-a35b-instruct"): {"context_window": 262_144},
}

# Preset-specific records win over the shared ``(kind, model_id)`` lookup so a
# gateway can claim its own identifiers without clashing with another provider
# serving a different upstream model under the same wire kind.
OFFICIAL_CATALOG_BY_PRESET = {
    ("nvidia-nim", "nvidia/llama-3.3-nemotron-super-49b-v1.5"): {"context_window": 131_072},
    ("nvidia-nim", "nvidia/llama-3.3-nemotron-super-49b-v1"): {"context_window": 131_072},
    ("nvidia-nim", "nvidia/nvidia-nemotron-nano-9b-v2"): {"context_window": 131_072},
    ("nvidia-nim", "nvidia/llama-3.1-nemotron-nano-8b-v1"): {"context_window": 131_072},
    ("nvidia-nim", "nvidia/llama-3.1-nemotron-ultra-253b-v1"): {"context_window": 128_000},
    ("nvidia-nim", "nvidia/llama-3.1-nemotron-70b-instruct"): {"context_window": 128_000},
    ("nvidia-nim", "nvidia/nemotron-3-super-120b-a12b"): {"context_window": 262_144},
    ("nvidia-nim", "nvidia/nemotron-3-nano-30b-a3b"): {"context_window": 131_072},
    ("nvidia-nim", "nvidia/nemotron-3.5-lightning-30b-a3b"): {"context_window": 262_144},
    ("nvidia-nim", "nvidia/nemotron-3-ultra-550b-a55b"): {"context_window": 1_000_000},
    ("nvidia-nim", "nvidia/nemotron-nano-12b-v2-vl"): {"context_window": 128_000},
}

# models.dev keys hosts under a vendor id (``nvidia``) rather than the wire
# kind, so a preset supplies its own lookup key when the kind alone misses.
MODELS_DEV_PROVIDER_KEYS = {
    "nvidia-nim": ("nvidia", "integrate.api.nvidia.com"),
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

MODELS_DEV_URL = "https://models.dev/api.json"
MODELS_DEV_TTL_SECONDS = 600
_catalog_cache: dict = {"value": None, "fetched_at": 0.0}


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
    # models.dev nests its window as ``limit.context``; some gateways send
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


async def _models_dev_catalog():
    """Fetch the models.dev catalog once per TTL; failure serves stale data.

    A sync resolves metadata for every model in a listing, so re-downloading
    the multi-megabyte catalog per model would dominate the sync (NVIDIA NIM
    alone lists 100+ models).  Failure must never block a provider sync.
    """
    now = time.monotonic()
    if _catalog_cache["value"] is not None and now - _catalog_cache["fetched_at"] < MODELS_DEV_TTL_SECONDS:
        return _catalog_cache["value"]
    try:
        async with httpx.AsyncClient(timeout=5, follow_redirects=False) as client:
            response = await client.get(MODELS_DEV_URL)
            response.raise_for_status()
            catalog = response.json()
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        return _catalog_cache["value"]
    if isinstance(catalog, dict):
        _catalog_cache["value"] = catalog
        _catalog_cache["fetched_at"] = now
        return catalog
    return _catalog_cache["value"]


async def models_dev(keys, model_id):
    """Look up a model in models.dev trying each provider key in order."""
    if isinstance(keys, str):
        keys = (keys,)
    catalog = await _models_dev_catalog()
    if not isinstance(catalog, dict):
        return {}
    providers = catalog.get("providers", catalog)
    if not isinstance(providers, dict):
        return {}
    row = {}
    for key in keys:
        candidate = providers.get(key, {})
        models = candidate.get("models", candidate) if isinstance(candidate, dict) else {}
        row = models.get(model_id, {}) if isinstance(models, dict) else {}
        if row:
            break
    if not isinstance(row, dict) or not row:
        return {}
    limit = row.get("limit") if isinstance(row.get("limit"), dict) else {}
    return {"context_window": context_limit(row),
            "max_output_tokens": row.get("max_output_tokens") or row.get("output") or limit.get("output"),
            "modalities": row.get("modalities"), "pricing": row.get("cost") or row.get("pricing")}


def _models_dev_keys(preset_id, kind):
    keys = list(MODELS_DEV_PROVIDER_KEYS.get(preset_id, ()))
    if kind not in keys:
        keys.append(kind)
    return tuple(keys)


async def resolve(kind, model_id, provider_item, resolver_ids=("provider_api", "official_catalog", "models_dev", "builtin"),
                  *, preset_id=None):
    """Return normalized effective metadata and all non-manual candidates."""
    candidates = {}
    api_values = {"context_window": context_limit(provider_item),
                  "max_output_tokens": provider_item.get("max_output_tokens") or provider_item.get("outputTokenLimit")}
    if "provider_api" in resolver_ids and any(value is not None for value in api_values.values()):
        candidates["provider_api"] = _record(api_values, "provider_api", "official")
    official = OFFICIAL_CATALOG_BY_PRESET.get((preset_id, model_id)) if preset_id else None
    official = official or OFFICIAL_CATALOG.get((kind, model_id), {})
    if "official_catalog" in resolver_ids and official:
        candidates["official_catalog"] = _record(official, "official_catalog", "official")
    external = await models_dev(_models_dev_keys(preset_id, kind), model_id) if "models_dev" in resolver_ids else {}
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
