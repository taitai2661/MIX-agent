"""Capability probing for provider models.

These helpers used to live in ``api.routes``; the run engine imported
``probe_due_models`` from there, which inverted the layering (domain -> API).
They depend only on the provider adapters, the capability-probe module, and the
database models, so they belong next to the providers.
"""

from sqlalchemy import select

from mix_agent import config
from mix_agent.auth.security import read_secret
from mix_agent.db.models import Model, Provider, Settings
from mix_agent.providers.adapters import Adapter
from mix_agent.providers import capability_probe


def tool_capability(model_data):
    """Return the effective tool capability, respecting explicit user overrides."""
    return capability_probe.capability(model_data, "tools")


def capability_targets(model_data, provider_kind):
    """Capabilities worth confirming automatically for one model."""
    return capability_probe.due_capabilities(model_data, provider_kind)


async def _capability_adapter(db, provider):
    return Adapter(provider.data, read_secret(db, provider.data.get("secret_id")))


async def verify_tool_capability(db, owner, model, provider, source):
    """Persist a safe, one-call tool support check without executing a tool."""
    try:
        adapter = await _capability_adapter(db, provider)
    except Exception as exc:  # noqa: BLE001 - classified; never leaks provider details
        model.data = capability_probe.merge(
            model.data, {"tools": "unknown"}, {"tools": type(exc).__name__}, source
        )
        return "unknown"
    model.data = await capability_probe.probe(
        adapter, model.data, ["tools"], source, provider.data.get("kind")
    )
    return model.data["tool_probe"]["status"]


async def verify_model_capabilities(db, owner, model, provider, source, capabilities=None):
    """Confirm the requested (or all due) capabilities and persist the verdicts."""
    requested = capabilities or capability_targets(model.data, provider.data.get("kind"))
    if not requested:
        return model.data
    try:
        adapter = await _capability_adapter(db, provider)
    except Exception as exc:  # noqa: BLE001 - classified; never leaks provider details
        model.data = capability_probe.merge(
            model.data,
            {capability: "unknown" for capability in requested},
            {capability: type(exc).__name__ for capability in requested},
            source,
        )
        return model.data
    model.data = await capability_probe.probe(
        adapter, model.data, requested, source, provider.data.get("kind")
    )
    return model.data


async def probe_due_models(db, limit=None):
    """Confirm due capabilities for Auto candidates across owners, bounded."""
    limit = config.PROBE_BATCH if limit is None else limit
    if limit <= 0:
        return 0
    seen, pending = set(), []
    for settings_row in db.scalars(select(Settings)):
        for model_id in settings_row.data.get("auto_model_ids", []):
            if model_id in seen:
                continue
            seen.add(model_id)
            model = db.get(Model, model_id)
            provider = db.get(Provider, model.data.get("provider_id")) if model else None
            if model is None or provider is None or provider.owner_id != settings_row.owner_id:
                continue
            due = capability_targets(model.data, provider.data.get("kind"))
            if due:
                pending.append((model, provider, due))
            if len(pending) >= limit:
                break
        if len(pending) >= limit:
            break
    confirmed = 0
    for model, provider, due in pending:
        try:
            await verify_model_capabilities(db, provider.owner_id, model, provider, "automatic", due)
            db.commit()
            confirmed += 1
        except Exception:  # noqa: BLE001 - one model must not abort the pass
            db.rollback()
    return confirmed


async def probe_auto_candidates(db, owner_id, allowed_ids, capabilities, limit=None):
    """Confirm the requested capabilities for Auto candidates before selecting.

    Only unknown capabilities are probed, so confirmed models cost nothing.
    """
    limit = config.PROBE_SELECT_BATCH if limit is None else limit
    confirmed = 0
    for model_id in allowed_ids:
        if confirmed >= limit:
            break
        model = db.get(Model, model_id)
        provider = db.get(Provider, model.data.get("provider_id")) if model else None
        if model is None or model.owner_id != owner_id or provider is None or provider.owner_id != owner_id:
            continue
        due = [
            capability
            for capability in capabilities
            if capability_probe.probe_needed(model.data, capability, provider.data.get("kind"))
        ]
        if not due:
            continue
        try:
            await verify_model_capabilities(db, owner_id, model, provider, "automatic", due)
            db.commit()
            confirmed += 1
        except Exception:  # noqa: BLE001 - probing must never block a request
            db.rollback()
    return confirmed
