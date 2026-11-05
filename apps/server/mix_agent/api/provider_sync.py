"""Provider model-catalogue synchronization.

Extracted from ``routes_providers`` so the HTTP module stays a thin surface.
Kept in ``api`` because it is only used by the provider routes and their tests.
"""

from sqlalchemy import select

from mix_agent.api.common import own
from mix_agent.auth.security import read_secret
from mix_agent.db.models import Model, Settings, now
from mix_agent.providers.adapters import Adapter
from mix_agent.providers.model_roles import is_auto_chat_eligible


async def sync_provider_models(db, owner_id, provider):
    """Fetch a provider catalog, preserve local model state, and seed Auto.

    This deliberately leaves models without a known context window out of
    Auto. They remain visible for a user to configure manually.
    """
    try:
        adapter = Adapter(
            {**provider.data, "id": provider.id},
            read_secret(db, provider.data.get("secret_id")),
        )
        fetched = await adapter.list_models()
    except Exception as exc:  # noqa: BLE001 - Provider errors must not undo a saved connection.
        error = str(exc) if str(exc).startswith("missing_extra_config:") else type(exc).__name__
        return {"status": "failed", "count": 0, "auto_count": 0, "error": error}

    existing = {
        m.data["model_id"]: m
        for m in db.scalars(select(Model).where(Model.owner_id == owner_id))
        if m.data["provider_id"] == provider.id
    }
    settings = own(db, Settings, "settings", owner_id)
    auto_ids = list(dict.fromkeys(settings.data.get("auto_model_ids", [])))
    auto_id_set = set(auto_ids)
    added_to_auto = 0
    for item in fetched:
        model = existing.get(item["model_id"])
        data = {**item, "provider_id": provider.id, "fetched_at": now().isoformat()}
        if model:
            data["overrides"] = model.data.get("overrides", {})
            # Manual context limits are deliberate safety limits. Preserve
            # them independently from API data so refreshes cannot erase them.
            manual_context = model.data.get("context_window_override")
            if manual_context is None and model.data.get("context_source") == "manual":
                manual_context = model.data.get("context_window")
            data["context_window_override"] = manual_context
            data["metadata_overrides"] = model.data.get("metadata_overrides", {})
            if "tool_probe" in model.data:
                data["tool_probe"] = model.data["tool_probe"]
        if data.get("context_window"):
            data["provider_context_window"] = data["context_window"]
            data["provider_context_source"] = data.get("context_source")
        if data.get("context_window_override"):
            data["context_window"] = data["context_window_override"]
            data["context_source"] = "manual"
            data["context_confidence"] = "runtime"
            data.setdefault("metadata", {})["context_window"] = {
                "value": data["context_window_override"],
                "source": "manual",
                "confidence": "runtime",
                "resolved_at": now().isoformat(),
            }
        if model:
            model.data = data
        else:
            model = Model(owner_id=owner_id, data=data)
            db.add(model)
            db.flush()
        if (
            data.get("context_window")
            and is_auto_chat_eligible(data, provider.data.get("kind"))
            and model.id not in auto_id_set
        ):
            auto_ids.append(model.id)
            auto_id_set.add(model.id)
            added_to_auto += 1

    # ``existing`` contains every previously synced model for this provider and
    # now points at its refreshed data. New special-purpose models were never
    # added above, so only retained IDs need cleanup here.
    provider_models_by_id = {model.id: model for model in existing.values()}
    auto_ids = [
        model_id
        for model_id in auto_ids
        if model_id not in provider_models_by_id
        or is_auto_chat_eligible(provider_models_by_id[model_id].data, provider.data.get("kind"))
    ]

    settings.data = {
        **settings.data,
        "auto_model_ids": auto_ids,
        "default_model_id": settings.data.get("default_model_id") or ("auto" if auto_ids else ""),
    }
    return {"status": "ok", "count": len(fetched), "auto_count": added_to_auto}
