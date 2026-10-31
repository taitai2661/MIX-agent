"""Provider/model connection endpoints.

Extracted from ``api.routes`` so provider setup (the largest cluster of
validation and network calls) has its own module.  The generic collection
routes in ``routes.py`` still expose model CRUD; the explicit provider routes
must stay registered before them so they keep precedence.
"""

from urllib.parse import urlsplit

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select

from mix_agent.api.common import audit, context, own, public
from mix_agent.api.schemas import ProviderInput, ProviderView
from mix_agent.auth.security import read_secret, store_secret
from mix_agent.db.models import Agent, Conversation, Model, Provider, Secret, Settings, now
from mix_agent.providers.adapters import DEFAULT_URLS, Adapter
from mix_agent.providers.catalog import CUSTOM_KINDS, KIND_VALUES, catalog, get_preset
from mix_agent.providers.model_roles import is_auto_chat_eligible

router = APIRouter(prefix="/api/v1")


async def provider_values(body, db, owner, previous=None):
    data = body.model_dump(exclude={"api_key", "kind"})
    preset = get_preset(body.preset_id) if body.preset_id else None
    if body.preset_id and not preset:
        raise HTTPException(422, "不明なProviderプリセットです")
    if preset:
        fields = {field["key"]: field for field in preset.get("extra_config_schema", [])}
        if any(key not in fields for key in body.extra_config):
            raise HTTPException(422, "このProviderでは使えない追加設定があります")
        if any(field.get("required") and not body.extra_config.get(key) for key, field in fields.items()):
            # Keep incomplete connections editable; sync/transport can explain the missing field.
            pass
    if preset and preset["id"] == "custom":
        if body.kind not in CUSTOM_KINDS:
            raise HTTPException(422, "カスタムはOpenAI互換・Anthropic・Geminiから選択してください")
        kind, url = body.kind, body.base_url
    elif preset:
        kind, url = preset["kind"], body.base_url or preset["default_url"]
    else:
        if body.kind not in KIND_VALUES:
            raise HTTPException(422, "Providerの接続方式を指定してください")
        kind, url = body.kind, body.base_url or DEFAULT_URLS[body.kind]
    if not url:
        raise HTTPException(422, "このProviderはBase URLを指定してください")
    data["kind"] = kind
    parsed = urlsplit(url)
    if (
        parsed.scheme not in ("http", "https")
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.query
        or parsed.fragment
    ):
        raise HTTPException(422, "有効なHTTP(S) Base URLを指定してください")
    if not body.allow_private:
        from mix_agent.tools.network import public_address

        try:
            await public_address(parsed.hostname, parsed.port or (443 if parsed.scheme == "https" else 80))
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            raise HTTPException(422, "LAN接続先は「プライベート接続を許可」を明示してください")
    data["base_url"] = url
    data["extra_config"] = body.extra_config
    old_secret_id = previous.get("secret_id") if previous else None
    if body.api_key is not None:
        data["secret_id"] = store_secret(db, owner, body.api_key, "provider") if body.api_key else None
        # Replacing or clearing a key must not leave the previous ciphertext
        # orphaned in the secrets table.
        if old_secret_id and old_secret_id != data["secret_id"]:
            old_secret = db.get(Secret, old_secret_id)
            if old_secret and old_secret.owner_id == owner:
                db.delete(old_secret)
    elif previous:
        data["secret_id"] = old_secret_id
    return data


async def sync_provider_models(db, owner_id, provider):
    """Fetch a provider catalog, preserve local model state, and seed Auto.

    This deliberately leaves models without a known context window out of
    Auto.  They remain visible for a user to configure manually.
    """
    try:
        adapter = Adapter(provider.data, read_secret(db, provider.data.get("secret_id")))
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
            # Manual context limits are deliberate safety limits.  Preserve
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


def provider_response(row, sync):
    response = public(row)
    response["model_sync"] = sync
    return response


@router.get("/providers")
def providers(ctx=Depends(context)):
    db, s = ctx
    return [public(r) for r in db.scalars(select(Provider).where(Provider.owner_id == s.owner_id))]


@router.get("/provider-presets")
def provider_presets(ctx=Depends(context)):
    # Require an authenticated admin session: this catalog describes outbound services.
    return catalog()


@router.post("/providers", response_model=ProviderView)
async def add_provider(body: ProviderInput, ctx=Depends(context)):
    db, s = ctx
    row = Provider(owner_id=s.owner_id, data=await provider_values(body, db, s.owner_id))
    db.add(row)
    db.flush()
    audit(db, s.owner_id, "provider.create", row.id)
    db.commit()
    sync = await sync_provider_models(db, s.owner_id, row)
    db.commit()
    return provider_response(row, sync)


@router.patch("/providers/{key}", response_model=ProviderView)
async def edit_provider(key: str, body: ProviderInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Provider, key, s.owner_id)
    previous = dict(row.data)
    row.data = await provider_values(body, db, s.owner_id, row.data)
    connection_changed = any(
        row.data.get(field) != previous.get(field)
        for field in ("kind", "base_url", "secret_id", "extra_config")
    )
    audit(db, s.owner_id, "provider.update", key)
    db.commit()
    sync = (
        await sync_provider_models(db, s.owner_id, row)
        if connection_changed
        else {
            "status": "skipped",
            "count": 0,
            "auto_count": 0,
        }
    )
    db.commit()
    return provider_response(row, sync)


@router.delete("/providers/{key}")
def delete_provider(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Provider, key, s.owner_id)
    models = list(
        db.scalars(
            select(Model).where(
                Model.owner_id == s.owner_id,
                Model.data["provider_id"].astext == key,
            )
        )
    )
    model_ids = {model.id for model in models}
    for model in models:
        db.delete(model)
    secret_id = row.data.get("secret_id")
    if secret_id:
        secret = db.get(Secret, secret_id)
        if secret and secret.owner_id == s.owner_id:
            db.delete(secret)
    settings = db.scalar(select(Settings).where(Settings.id == "settings", Settings.owner_id == s.owner_id))
    if settings:
        auto_ids = [mid for mid in settings.data.get("auto_model_ids", []) if mid not in model_ids]
        default_model_id = settings.data.get("default_model_id", "")
        if default_model_id in model_ids:
            default_model_id = "auto" if auto_ids else ""
        settings.data = {**settings.data, "auto_model_ids": auto_ids, "default_model_id": default_model_id}
    if model_ids:
        for agent in db.scalars(
            select(Agent).where(
                Agent.owner_id == s.owner_id,
                Agent.data["model_id"].astext.in_(model_ids),
            )
        ):
            agent.data = {**agent.data, "model_id": ""}
        for conversation in db.scalars(
            select(Conversation).where(
                Conversation.owner_id == s.owner_id,
                Conversation.data["selection"]["model_id"].astext.in_(model_ids),
            )
        ):
            selection = conversation.data.get("selection")
            if isinstance(selection, dict) and selection.get("model_id") in model_ids:
                conversation.data = {**conversation.data, "selection": {**selection, "model_id": "auto"}}
    db.delete(row)
    audit(db, s.owner_id, "provider.delete", key)
    db.commit()
    return {"ok": True}


@router.post("/providers/{key}/{action}")
async def provider_action(key: str, action: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Provider, key, s.owner_id)
    if action == "test":
        try:
            return await Adapter(row.data, read_secret(db, row.data.get("secret_id"))).test_connection()
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            raise HTTPException(502, "接続テストに失敗しました。")
    if action != "sync-models":
        raise HTTPException(404)
    result = await sync_provider_models(db, s.owner_id, row)
    if result["status"] != "ok":
        raise HTTPException(502, "モデル一覧を取得できません。既存設定は保持されています。")
    db.commit()
    return {"ok": True, **result}
