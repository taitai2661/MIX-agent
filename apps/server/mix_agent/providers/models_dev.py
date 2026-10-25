"""Lookup of the models.dev catalog: specs, pricing, capabilities.

MIX keeps no model table of its own.  Context windows, prices, modalities and
capabilities are resolved from the published models.dev ``api.json`` during a
provider sync and stored as evidence on the model record.  ``MIX_MODELS_DEV_URL``
accepts the published address or a local ``api.json`` path, so development never
needs a network round trip.

Failures never block a sync: a cached catalog keeps serving until its TTL
expires, and an unreachable source simply leaves the facts unknown.
"""

from __future__ import annotations

import json
import re
import time
from pathlib import Path

import httpx

from mix_agent import config

TTL_SECONDS = 600
SOURCE = "models_dev"

# Registered preset -> models.dev provider ids.  Only listed ids are looked up;
# an unlisted preset means "no catalog for this provider", never a guess.
PROVIDER_KEYS = {
    "openai": ("openai",),
    "anthropic": ("anthropic",),
    "gemini": ("google",),
    "openrouter": ("openrouter",),
    "groq": ("groq",),
    "xai": ("xai",),
    "mistral": ("mistral",),
    "deepseek": ("deepseek",),
    "together": ("togetherai",),
    "fireworks": ("fireworks-ai",),
    "perplexity": ("perplexity",),
    "cerebras": ("cerebras",),
    "sambanova": ("sambanova",),
    "nvidia-nim": ("nvidia",),
    "ai21": ("ai21",),
    "cohere": ("cohere",),
    "huggingface": ("huggingface",),
    "cloudflare": ("cloudflare-workers-ai",),
    "vercel-ai-gateway": ("vercel",),
    "helicone": ("helicone",),
    "deepinfra": ("deepinfra",),
    "nebius": ("nebius",),
    "novita": ("novita-ai",),
    "chutes": ("chutes",),
    "siliconflow": ("siliconflow",),
    "modelscope": ("modelscope",),
    "alibaba-model-studio": ("alibaba-cn",),
    "moonshot": ("moonshotai",),
    "zhipu": ("zhipuai",),
    "minimax": ("minimax",),
    "bytedance-ark": ("volcengine",),
    "opencode-zen": ("opencode",),
    "opencode-go": ("opencode-go",),
}

_SNAPSHOT_SUFFIX = re.compile(r"-\d{4}$")
_ROW_FIELDS = ("limit", "modalities", "cost", "tool_call", "reasoning", "structured_output")
_catalog_cache: dict[str, dict] = {}


def provider_keys(preset_id):
    return PROVIDER_KEYS.get(preset_id, ())


def _variants(model_id):
    """Id spellings that may name the same model: exact, no vendor prefix,
    no snapshot suffix, lower-cased.  Mirrors the rewrites models.dev's own
    collectors apply so one model is not recorded under several ids."""
    forms = {model_id.lower()}
    if "/" in model_id:
        forms.add(model_id.split("/", 1)[1].lower())
    return {form for item in forms for form in (item, _SNAPSHOT_SUFFIX.sub("", item))}


def _positive_int(value):
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        return None
    return value


def _compact(payload):
    provider_ids = {item for keys in PROVIDER_KEYS.values() for item in keys}
    index = {}
    for provider_id in provider_ids:
        provider = payload.get(provider_id)
        models = provider.get("models") if isinstance(provider, dict) else None
        if not isinstance(models, dict):
            continue
        rows = {}
        for model_id, row in models.items():
            if isinstance(row, dict):
                rows[model_id] = {key: row.get(key) for key in _ROW_FIELDS}
        index[provider_id] = rows
    return index


def _read_local(location):
    path = Path(location)
    if path.is_dir():
        path = path / "api.json"
    try:
        payload = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    return payload if isinstance(payload, dict) else None


async def _fetch():
    location = config.MODELS_DEV_URL
    if location.startswith(("http://", "https://")):
        try:
            async with httpx.AsyncClient(timeout=30, follow_redirects=False) as client:
                response = await client.get(location)
                response.raise_for_status()
                payload = response.json()
        except Exception:  # noqa: BLE001 - classified; a catalog outage must not fail a sync
            return None
        return _compact(payload) if isinstance(payload, dict) else None
    payload = _read_local(location)
    return _compact(payload) if payload is not None else None


async def catalog():
    """Return the compacted models.dev index, refreshed once per TTL."""
    now = time.monotonic()
    cached = _catalog_cache.get("catalog")
    if cached is not None and now - cached["fetched_at"] < TTL_SECONDS:
        return cached["value"]
    value = await _fetch()
    if value is None:
        return cached["value"] if cached is not None else None
    _catalog_cache["catalog"] = {"value": value, "fetched_at": now}
    return value


def _match(rows, model_id):
    if model_id in rows:
        return model_id
    wanted = _variants(model_id)
    for key in rows:
        if wanted & _variants(key):
            return key
    return None


def _normalize(row):
    limit = row.get("limit") if isinstance(row.get("limit"), dict) else {}
    modalities = row.get("modalities")
    if not isinstance(modalities, dict) or not modalities:
        modalities = None
    inputs = modalities.get("input") if modalities else None
    capabilities = {
        "tools": row.get("tool_call"),
        "vision": "image" in inputs if isinstance(inputs, list) else None,
        "reasoning": row.get("reasoning"),
        "structured_output": row.get("structured_output"),
    }
    pricing = row.get("cost")
    return {
        "context_window": _positive_int(limit.get("context")) or _positive_int(limit.get("input")),
        "max_output_tokens": _positive_int(limit.get("output")),
        "modalities": modalities,
        "capabilities": capabilities if any(value is not None for value in capabilities.values()) else None,
        "pricing": pricing if isinstance(pricing, dict) and pricing else None,
    }


async def lookup(provider_ids, model_id):
    """Return normalized models.dev facts for ``model_id``, or ``{}``."""
    if isinstance(provider_ids, str):
        provider_ids = (provider_ids,)
    if not model_id:
        return {}
    index = await catalog()
    if index is None:
        return {}
    for provider_id in provider_ids:
        rows = index.get(provider_id)
        if not isinstance(rows, dict):
            continue
        key = _match(rows, model_id)
        if key is not None:
            return _normalize(rows[key])
    return {}
