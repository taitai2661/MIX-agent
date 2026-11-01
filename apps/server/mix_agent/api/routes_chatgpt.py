"""Unofficial ChatGPT / Codex subscription endpoints.

Kept out of ``routes_providers`` so the provider module does not grow into a
second monolith. The router is mounted **before** the generic provider router
so ``POST /providers/{key}/logout`` is not captured by ``/{key}/{action}``.

Every endpoint here is opt-in and returns stable error codes. Tokens never
appear in a response: the device flow persists them through the encrypted
Secret store only.
"""

from fastapi import APIRouter, Depends, HTTPException

from mix_agent.api.common import audit, context, own
from mix_agent.api.schemas import (
    ChatGptDeviceStart,
    ChatGptDeviceStartView,
    ChatGptDeviceStatusView,
    QuotaView,
)
from mix_agent.auth.security import read_secret
from mix_agent.db.models import Provider
from mix_agent.providers import chatgpt
from mix_agent.providers.adapters import Adapter

router = APIRouter(prefix="/api/v1")


def _require_chatgpt(row) -> None:
    if row.data.get("preset_id") != "chatgpt":
        raise HTTPException(422, "ChatGPT 連携のProviderではありません")


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
    if previous:
        for key in ("auth", "secret_id", "status", "billing_mode"):
            if key in previous:
                data[key] = previous[key]
    return data


@router.post("/providers/chatgpt/device/start", response_model=ChatGptDeviceStartView)
async def chatgpt_device_start(body: ChatGptDeviceStart, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Provider, body.provider_id, s.owner_id)
    _require_chatgpt(row)
    try:
        started = await chatgpt.start_device_login(row.id, s.owner_id)
    except chatgpt.ChatGptError as exc:
        raise HTTPException(502, exc.code) from exc
    audit(db, s.owner_id, "provider.chatgpt_login_start", row.id)
    db.commit()
    return started


@router.get("/providers/chatgpt/device/{login_id}", response_model=ChatGptDeviceStatusView)
async def chatgpt_device_status(login_id: str, ctx=Depends(context)):
    db, s = ctx
    state = chatgpt.device_login_state(login_id, s.owner_id)
    if state is None:
        raise HTTPException(404, "chatgpt_login_not_found")
    try:
        result = await chatgpt.poll_device_login(login_id, s.owner_id)
    except chatgpt.ChatGptError as exc:
        raise HTTPException(404, exc.code) from exc
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
        raise HTTPException(404, "chatgpt_login_not_found")
    return {"ok": True}


async def _quota_for(db, row, force: bool) -> dict:
    _require_chatgpt(row)
    adapter = Adapter(row.data, read_secret(db, row.data.get("secret_id")))
    try:
        quota = await adapter.quota(force=force)
    except chatgpt.ChatGptError as exc:
        status = 401 if exc.code in {chatgpt.AUTH_REQUIRED, chatgpt.AUTH_EXPIRED} else 502
        raise HTTPException(status, exc.code) from exc
    if quota is None:
        raise HTTPException(400, "chatgpt_quota_unavailable")
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
