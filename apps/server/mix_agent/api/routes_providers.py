"""Provider/model connection endpoints.

Extracted from ``api.routes`` so provider setup (the largest cluster of
validation and network calls) has its own module.  The generic collection
routes in ``routes.py`` still expose model CRUD; the explicit provider routes
must stay registered before them so they keep precedence.
"""

import logging
from urllib.parse import urlsplit

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select

from mix_agent.api.common import audit, context, own, public
from mix_agent.api.provider_sync import sync_provider_models
from mix_agent.api.schemas import ProviderInput, ProviderView
from mix_agent.auth.security import read_secret, store_secret
from mix_agent.db.models import Agent, Conversation, Model, Provider, Secret, Settings
from mix_agent.providers import chatgpt
from mix_agent.providers.adapters import DEFAULT_URLS, Adapter
from mix_agent.providers.catalog import CUSTOM_KINDS, KIND_VALUES, catalog, get_preset

router = APIRouter(prefix="/api/v1")
logger = logging.getLogger(__name__)


def _chatgpt_credentials(db, data) -> chatgpt.ChatGptCredentials | None:
    """Read the OAuth blob before a delete/switch removes the secret row.

    Best-effort: a corrupted or unreadable secret must not block deleting the
    provider. The local row is still removed; only remote revocation is lost.
    """
    if data.get("preset_id") != "chatgpt" or not data.get("secret_id"):
        return None
    try:
        return chatgpt.ChatGptCredentials.from_json(read_secret(db, data["secret_id"]))
    except Exception:  # noqa: BLE001 - revocation is best-effort
        return None


async def _revoke_chatgpt(creds) -> None:
    if creds is None:
        return
    try:
        await chatgpt.revoke_credentials(creds)
    except Exception as err:  # noqa: BLE001 - local delete must still complete
        logger.warning("could not revoke the ChatGPT credentials: %s", type(err).__name__)


async def provider_values(body, db, owner, previous=None, preset_id=None):
    # `acknowledge_unofficial` is a one-shot consent flag, not provider state;
    # persisting it would leak a UI decision into backups and API responses.
    data = body.model_dump(exclude={"api_key", "kind", "acknowledge_unofficial"})
    # PATCH is partial: omitted fields keep their stored values instead of the
    # model defaults, which used to clear a custom base URL, extra config, LAN
    # opt-in and rate limit on a rename.
    if previous:
        for key in ("base_url", "extra_config", "allow_private", "rate_limit_rpm", "rate_limit_period"):
            if key not in body.model_fields_set and key in previous:
                data[key] = previous[key]
    # PATCH may omit `preset_id`; preserving the stored one keeps a partial
    # update from silently downgrading a ChatGPT row to a normal provider.
    preset_id = preset_id if preset_id is not None else body.preset_id
    data["preset_id"] = preset_id
    preset = get_preset(preset_id) if preset_id else None
    if preset_id and not preset:
        raise HTTPException(422, "不明なProviderプリセットです")
    extra_config = data["extra_config"]
    if preset:
        fields = {field["key"]: field for field in preset.get("extra_config_schema", [])}
        if any(key not in fields for key in extra_config):
            raise HTTPException(422, "このProviderでは使えない追加設定があります")
        if any(field.get("required") and not extra_config.get(key) for key, field in fields.items()):
            # Keep incomplete connections editable; sync/transport can explain the missing field.
            pass
    # `kind` is not persisted for preset providers (the preset owns it), but a
    # legacy/custom row may omit it on PATCH. Fall back to the stored kind so a
    # rename does not require resending it.
    kind = body.kind if "kind" in body.model_fields_set else (previous or {}).get("kind")
    if preset and preset["id"] == "custom":
        if kind not in CUSTOM_KINDS:
            raise HTTPException(422, "カスタムはOpenAI互換・Anthropic・Geminiから選択してください")
        url = data["base_url"]
    elif preset:
        kind = preset["kind"]
        # ChatGPT tokens must never be sent to a caller-supplied host; the
        # helper below rejects an override and pins the Codex backend URL.
        url = (
            preset["default_url"]
            if preset["id"] == "chatgpt"
            else (data["base_url"] or preset["default_url"])
        )
    else:
        if kind not in KIND_VALUES:
            raise HTTPException(422, "Providerの接続方式を指定してください")
        url = data["base_url"] or DEFAULT_URLS[kind]
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
    if not data["allow_private"]:
        from mix_agent.tools.network import public_address

        try:
            await public_address(parsed.hostname, parsed.port or (443 if parsed.scheme == "https" else 80))
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            raise HTTPException(422, "LAN接続先は「プライベート接続を許可」を明示してください")
    data["base_url"] = url
    data["extra_config"] = extra_config
    if preset and preset["id"] == "chatgpt":
        from mix_agent.api.routes_chatgpt import chatgpt_provider_values

        return chatgpt_provider_values(data, body, preset, previous)
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
        # An OAuth blob is not an API key. Switching a ChatGPT row to a normal
        # provider must start without a secret instead of reusing the token.
        data["secret_id"] = None if previous.get("preset_id") == "chatgpt" else old_secret_id
    return data


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
    if body.preset_id == "chatgpt":
        from mix_agent.api.routes_chatgpt import chatgpt_enabled

        if not chatgpt_enabled(db, s.owner_id):
            raise HTTPException(422, "ChatGPT 連携は設定で有効化してください")
        if not body.acknowledge_unofficial:
            raise HTTPException(422, "ChatGPT 連携は非公式・自己責任です。同意のうえで作成してください。")
    row = Provider(owner_id=s.owner_id, data=await provider_values(body, db, s.owner_id))
    db.add(row)
    db.flush()
    audit(db, s.owner_id, "provider.create", row.id)
    db.commit()
    # ChatGPT has no credential until the device-code flow completes, so the
    # immediate catalog fetch can only fail. The UI calls sync-models after
    # login instead.
    if row.data.get("preset_id") == "chatgpt":
        sync = {"status": "skipped", "count": 0, "auto_count": 0}
    else:
        sync = await sync_provider_models(db, s.owner_id, row)
    db.commit()
    return provider_response(row, sync)


@router.patch("/providers/{key}", response_model=ProviderView)
async def edit_provider(key: str, body: ProviderInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Provider, key, s.owner_id)
    previous = dict(row.data)
    old_chatgpt_creds = _chatgpt_credentials(db, previous)
    # Omitted `preset_id` means "keep the current preset"; an explicit null
    # clears it. Without this a PATCH that only renames a ChatGPT row used to
    # rewrite it as a normal provider and delete its OAuth secret.
    preset_id = body.preset_id if "preset_id" in body.model_fields_set else previous.get("preset_id")
    # Creating a ChatGPT provider requires explicit consent. Editing an
    # existing one must not become a back door that switches an ordinary
    # provider to the unofficial integration without that acknowledgement.
    if preset_id == "chatgpt" and previous.get("preset_id") != "chatgpt":
        from mix_agent.api.routes_chatgpt import chatgpt_enabled

        if not chatgpt_enabled(db, s.owner_id):
            raise HTTPException(422, "ChatGPT 連携は設定で有効化してください")
        if not body.acknowledge_unofficial:
            raise HTTPException(422, "ChatGPT 連携は非公式・自己責任です。同意のうえで保存してください。")
    row.data = await provider_values(body, db, s.owner_id, row.data, preset_id=preset_id)
    # A preset transition replaces the credential type. Delete the old secret
    # when the row no longer points at it so an API key cannot survive inside a
    # ChatGPT row (or an OAuth token inside an API-key row).
    old_secret_id = previous.get("secret_id")
    new_secret_id = row.data.get("secret_id")
    if old_secret_id and old_secret_id != new_secret_id:
        # A preset switch replaces the credential type. Revoke the ChatGPT
        # token before deleting its ciphertext so the refresh token does not
        # stay usable on OpenAI's side.
        await _revoke_chatgpt(old_chatgpt_creds)
        old_secret = db.get(Secret, old_secret_id)
        if old_secret and old_secret.owner_id == s.owner_id:
            db.delete(old_secret)
    connection_changed = any(
        row.data.get(field) != previous.get(field)
        for field in ("kind", "base_url", "secret_id", "extra_config")
    )
    audit(db, s.owner_id, "provider.update", key)
    db.commit()
    # ChatGPT has no credential until the device flow completes, so a sync
    # here can only fail. The UI calls sync-models after login instead, just as
    # the create path already does.
    sync = (
        await sync_provider_models(db, s.owner_id, row)
        if connection_changed and row.data.get("preset_id") != "chatgpt"
        else {
            "status": "skipped",
            "count": 0,
            "auto_count": 0,
        }
    )
    db.commit()
    return provider_response(row, sync)


@router.delete("/providers/{key}")
async def delete_provider(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Provider, key, s.owner_id)
    revoked = _chatgpt_credentials(db, row.data)
    await _revoke_chatgpt(revoked)
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
            return await Adapter(
                {**row.data, "id": row.id},
                read_secret(db, row.data.get("secret_id")),
            ).test_connection()
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            raise HTTPException(502, "接続テストに失敗しました。")
    if action != "sync-models":
        raise HTTPException(404)
    result = await sync_provider_models(db, s.owner_id, row)
    if result["status"] != "ok":
        raise HTTPException(502, "モデル一覧を取得できません。既存設定は保持されています。")
    db.commit()
    return {"ok": True, **result}
