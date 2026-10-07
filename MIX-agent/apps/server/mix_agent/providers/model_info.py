"""Lookup of the model-info static catalog: specs, pricing, capabilities.

MIX keeps no model table of its own.  Context windows, prices, modalities and
capabilities are resolved from the model-info JSON API during a provider sync
and stored as evidence on the model record.  ``MIX_MODEL_INFO_URL`` accepts an
``https://`` address (the published GitHub Pages site) or a local directory
holding a model-info checkout, so development never needs a network round
trip.

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

# Registered preset -> model-info provider ids.  Only listed ids are looked up;
# an unlisted preset means "no catalog for this provider", never a guess.
PROVIDER_KEYS = {
    "openai": ("openai",),
    "anthropic": ("anthropic",),
    "gemini": ("google",),
    "openrouter": ("openrouter",),
    "deepseek": ("deepseek",),
    "mistral": ("mistral",),
    "groq": ("groq",),
    "together": ("together",),
    "fireworks": ("fireworks",),
    "nvidia-nim": ("nvidia",),
    "xai": ("xai",),
    "minimax": ("minimax",),
    "moonshot": ("moonshot",),
    "zhipu": ("zai",),
    "opencode-zen": ("opencode",),
    "opencode-go": ("opencode-go",),
}

_SNAPSHOT_SUFFIX = re.compile(r"-\d{4}$")
_catalog_cache: dict[str, dict] = {}


def provider_keys(preset_id):
    return PROVIDER_KEYS.get(preset_id, ())


def _variants(model_id):
    """Id spellings that may name the same model: exact, no vendor prefix,
    no snapshot suffix, lower-cased.  Mirrors the rewrites model-info's own
    collectors apply so one model is not recorded under several ids."""
    forms = {model_id.lower()}
    if "/" in model_id:
        forms.add(model_id.split("/", 1)[1].lower())
    return {form for item in forms for form in (item, _SNAPSHOT_SUFFIX.sub("", item))}


def _split(entry):
    """Return ``(relationship, model)`` for either published shape.

    ``v1/providers/{id}/models.json`` bundles ``{model, relationship}`` rows
    while hand-written relationship files are the bare relationship object
    with an optional embedded ``model``.
    """
    if isinstance(entry.get("relationship"), dict):
        model = entry.get("model") if isinstance(entry.get("model"), dict) else {}
        return entry["relationship"], model
    return entry, entry.get("model") if isinstance(entry.get("model"), dict) else {}


def _match(entries, model_id):
    pairs = [_split(entry) for entry in entries]
    for pair in pairs:
        rel, model = pair
        if model_id in (rel.get("model_id"), model.get("model_id")):
            return pair
    wanted = _variants(model_id)
    for pair in pairs:
        rel, model = pair
        if any(wanted & _variants(str(value or ""))
               for value in (rel.get("model_id"), model.get("model_id"))):
            return pair
    return None


def _field(entry, model, key):
    """Relationship value wins over the model document.

    An omitted key inherits the model value, an explicit ``null`` means the
    fact is unknown at this provider, and a present object replaces the model
    object wholesale.
    """
    if key in entry:
        return entry[key]
    return model.get(key)


def _mapped_capabilities(raw):
    if not isinstance(raw, dict):
        return None
    tools = raw.get("tool_use")
    if tools is None:
        tools = raw.get("function_calling")
    mapped = {
        "tools": tools,
        "vision": raw.get("vision"),
        "reasoning": raw.get("reasoning"),
        "structured_output": raw.get("structured_output"),
    }
    return mapped if any(value is not None for value in mapped.values()) else None


def _normalize(pair):
    rel, model = pair
    context = _field(rel, model, "context")
    context = context if isinstance(context, dict) else {}
    modalities = _field(rel, model, "modalities")
    modalities = modalities if isinstance(modalities, dict) else None
    capabilities = _mapped_capabilities(_field(rel, model, "capabilities"))
    if isinstance(modalities, dict) and "image" in (modalities.get("input") or []):
        # Documented image input settles vision even when the capability row
        # is unknown; absence of image input never settles it.
        if capabilities is None:
            capabilities = {"tools": None, "vision": True, "reasoning": None, "structured_output": None}
        elif capabilities.get("vision") is None:
            capabilities["vision"] = True
    pricing = _field(rel, model, "pricing")
    return {
        "context_window": context.get("window"),
        "max_output_tokens": context.get("max_output_tokens"),
        "modalities": modalities,
        "capabilities": capabilities,
        "pricing": pricing if isinstance(pricing, dict) else None,
    }


async def _fetch(provider_id):
    location = config.MODEL_INFO_URL
    path = f"providers/{provider_id}/models.json"
    if location.startswith(("http://", "https://")):
        try:
            async with httpx.AsyncClient(timeout=5, follow_redirects=False) as client:
                response = await client.get(f"{location.rstrip('/')}/{path}")
                response.raise_for_status()
                payload = response.json()
        except Exception:  # noqa: BLE001 - classified; a catalog outage must not fail a sync
            return None
        return payload if isinstance(payload, list) else None
    try:
        payload = json.loads((Path(location) / path).read_text())
    except (OSError, ValueError):
        return None
    return payload if isinstance(payload, list) else None


async def provider_catalog(provider_id):
    """Return one provider's model rows, refreshed once per TTL."""
    now = time.monotonic()
    cached = _catalog_cache.get(provider_id)
    if cached is not None and now - cached["fetched_at"] < TTL_SECONDS:
        return cached["value"]
    value = await _fetch(provider_id)
    if value is None:
        return cached["value"] if cached is not None else None
    _catalog_cache[provider_id] = {"value": value, "fetched_at": now}
    return value


async def lookup(provider_ids, model_id):
    """Return normalized model-info facts for ``model_id``, or ``{}``."""
    if isinstance(provider_ids, str):
        provider_ids = (provider_ids,)
    if not model_id:
        return {}
    for provider_id in provider_ids:
        entries = await provider_catalog(provider_id)
        if not isinstance(entries, list):
            continue
        pair = _match(entries, model_id)
        if pair is not None:
            return _normalize(pair)
    return {}
