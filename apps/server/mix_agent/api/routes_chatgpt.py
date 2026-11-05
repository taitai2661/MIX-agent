"""Unofficial ChatGPT / Codex subscription endpoints.

Kept out of ``routes_providers`` so the provider module does not grow into a
second monolith. The router is mounted **before** the generic provider router
so ``POST /providers/{key}/logout`` is not captured by ``/{key}/{action}``.

Every endpoint here is opt-in and returns stable error codes. Tokens never
appear in a response: the device flow persists them through the encrypted
Secret store only.
"""

import logging

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select

from mix_agent.api.common import audit, context, own
from mix_agent.api.error_codes import APIError
from mix_agent.api.schemas import (
    ChatGptDeviceStart,
    ChatGptDeviceStartView,
    ChatGptDeviceStatusView,
    QuotaView,
)
from mix_agent.auth.security import read_secret
from mix_agent.db.models import Provider, Settings
from mix_agent.providers import chatgpt
from mix_agent.providers.adapters import Adapter

router = APIRouter(prefix="/api/v1")
logger = logging.getLogger(__name__)


def chatgpt_enabled(db, owner_id: str) -> bool:
    """Whether the operator opted into the unofficial integration.

    Existing rows keep working (and can be logged out) when this is off; only
    creating a new row, switching an existing one, and starting a login are
    gated. That keeps the kill switch from trapping credentials.
    """
    row = db.scalar(select(Settings).where(Settings.id == "settings", Settings.owner_id == owner_id))
    return bool(row and row.data.get("chatgpt_enabled") is True)


def _require_chatgpt(row) -> None:
    if row.data.get("preset_id") != "chatgpt":
        raise HTTPException(422, "ChatGPT 連携のProviderではありません")


def _chatgpt_http_error(exc: chatgpt.ChatGptError) -> APIError:
    """Translate an adapter error into a stable, translatable API error.

    Returning ``exc.code`` as ``detail`` left the English UI with a generic
    ``common.http_502`` because the code was not in ``ERROR_CODES``. Every
    branch below names a literal code so ``check-error-i18n.py`` can verify the
    table and the English translation stay in sync.
    """
    match exc.code:
        case "chatgpt_auth_required":
            return APIError(401, "provider.chatgpt_auth_required")
        case "chatgpt_auth_expired":
            return APIError(401, "provider.chatgpt_auth_expired")
        case "chatgpt_device_unavailable":
            return APIError(422, "provider.chatgpt_device_unavailable")
        case "chatgpt_device_failed":
            return APIError(502, "provider.chatgpt_device_failed")
        case "chatgpt_login_failed":
            return APIError(502, "provider.chatgpt_login_failed")
        case "chatgpt_login_expired":
            return APIError(410, "provider.chatgpt_login_expired")
        case "chatgpt_login_not_found":
            return APIError(404, "provider.chatgpt_login_not_found")
        case "chatgpt_quota_failed":
            return APIError(502, "provider.chatgpt_quota_failed")
        case "chatgpt_rate_limited":
            return APIError(429, "provider.chatgpt_rate_limited")
        case "chatgpt_refresh_failed":
            return APIError(502, "provider.chatgpt_refresh_failed")
        case "chatgpt_credentials_save_failed":
            return APIError(500, "provider.chatgpt_credentials_save_failed")
        case _:
            logger.warning("unclassified ChatGPT error: %s", exc.code)
            return APIError(502, "provider.chatgpt_failed")


def chatgpt_provider_values(data: dict, body, preset: dict, previous: dict | None) -> dict:
    """Normalize a ChatGPT provider row without touching OAuth credentials.

    The token secret is created only by the device-code flow. Editing the row
    (name, rate limit, …) must preserve an existing login, and an OAuth token
    must never be pointed at a caller-supplied host.
    """
    if body.base_url and body.base_url.rstrip("/") != preset["default_url"]:
        raise HTTPException(422, "ChatGPT 連携の接続先は変更できません")
    data["base_url"] = preset["default_url"]
    data["billing_mode"] = "subscription"
    data["status"] = "needs_login"
    # Only a previous ChatGPT row owns an OAuth secret. Carrying a normal
    # provider's API-key `secret_id` into this row would make the ChatGPT
    # adapter try to parse an API key as credentials and leave the old secret
    # orphaned.
    if previous and previous.get("preset_id") == "chatgpt":
        for key in ("auth", "secret_id", "status", "billing_mode"):
            if key in previous:
                data[key] = previous[key]
    # A new ChatGPT row (or one switched from another preset) has no credential
    # until the device flow completes. Keep the key present so the API response
    # always carries `has_secret_id: false` instead of omitting it.
    data.setdefault("secret_id", None)
    return data


@router.post("/providers/chatgpt/device/start", response_model=ChatGptDeviceStartView)
async def chatgpt_device_start(body: ChatGptDeviceStart, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Provider, body.provider_id, s.owner_id)
    _require_chatgpt(row)
    if not chatgpt_enabled(db, s.owner_id):
        raise HTTPException(422, "ChatGPT 連携は設定で有効化してください")
    try:
        started = await chatgpt.start_device_login(row.id, s.owner_id)
    except chatgpt.ChatGptError as exc:
        raise _chatgpt_http_error(exc) from exc
    audit(db, s.owner_id, "provider.chatgpt_login_start", row.id)
    db.commit()
    return started


@router.get("/providers/chatgpt/device/{login_id}", response_model=ChatGptDeviceStatusView)
async def chatgpt_device_status(login_id: str, ctx=Depends(context)):
    db, s = ctx
    state = chatgpt.device_login_state(login_id, s.owner_id)
    if state is None:
        raise HTTPException(404, "ChatGPT のログイン情報が見つかりません。もう一度ログインしてください。")
    try:
        result = await chatgpt.poll_device_login(login_id, s.owner_id)
    except chatgpt.ChatGptError as exc:
        raise _chatgpt_http_error(exc) from exc
    if result.get("status") == "complete":
        row = own(db, Provider, state["provider_id"], s.owner_id)
        creds = result.pop("credentials", None)
        if creds is not None:
            chatgpt.save_credentials(db, row, creds, s.owner_id)
            audit(db, s.owner_id, "provider.chatgpt_login", row.id)
            db.commit()
    return result


@router.post("/providers/chatgpt/device/{login_id}/cancel")
async def chatgpt_device_cancel(login_id: str, ctx=Depends(context)):
    _, s = ctx
    if not chatgpt.cancel_device_login(login_id, s.owner_id):
        raise HTTPException(404, "ChatGPT のログイン情報が見つかりません。もう一度ログインしてください。")
    return {"ok": True}


async def _quota_for(db, row, force: bool) -> dict:
    _require_chatgpt(row)
    adapter = Adapter(
        {**row.data, "id": row.id},
        read_secret(db, row.data.get("secret_id")),
    )
    try:
        quota = await adapter.quota(force=force)
    except chatgpt.ChatGptError as exc:
        raise _chatgpt_http_error(exc) from exc
    if quota is None:
        raise HTTPException(400, "ChatGPT の使用枠を取得できませんでした。")
    return quota


@router.get("/providers/{key}/quota", response_model=QuotaView)
async def provider_quota(key: str, ctx=Depends(context)):
    db, s = ctx
    return await _quota_for(db, own(db, Provider, key, s.owner_id), False)


@router.post("/providers/{key}/quota/refresh", response_model=QuotaView)
async def provider_quota_refresh(key: str, ctx=Depends(context)):
    db, s = ctx
    return await _quota_for(db, own(db, Provider, key, s.owner_id), True)


@router.post("/providers/{key}/logout")
async def provider_logout(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Provider, key, s.owner_id)
    _require_chatgpt(row)
    creds = chatgpt.ChatGptCredentials.from_json(read_secret(db, row.data.get("secret_id")))
    await chatgpt.revoke_credentials(creds)
    chatgpt.clear_credentials(db, row)
    audit(db, s.owner_id, "provider.chatgpt_logout", row.id)
    db.commit()
    return {"ok": True}
