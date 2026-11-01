"""Unofficial ChatGPT / Codex subscription support.

This module talks to the same private backend the Codex CLI uses. It is **not**
an OpenAI public API: endpoints, headers, and token semantics can change or be
blocked without notice. Use is at the operator's own risk and is disabled
unless the provider is explicitly configured.

Only token metadata is persisted, and only through the existing encrypted
``Secret`` store. Tokens must never be logged.
"""

from __future__ import annotations

import asyncio
import base64
import json
import time
import uuid
from dataclasses import asdict, dataclass
from datetime import UTC, datetime

import httpx

from mix_agent.auth.security import encrypt, store_secret
from mix_agent.db.models import Secret
from mix_agent.providers.chatgpt_config import (
    AUTH_EXPIRED,
    AUTH_ISSUER,
    AUTH_REQUIRED,
    CHATGPT_USAGE_URL,
    CLIENT_ID,
    DEVICE_REDIRECT_URI,
    DEVICE_TTL_SECONDS,
    DEVICE_VERIFICATION_URL,
    LOGIN_POLL_FLOOR_SECONDS,
    ORIGINATOR,
    ORIGINATOR_ENV,
    QUOTA_TTL_SECONDS,
    REFRESH_SKEW_SECONDS,
)

_HTTP_TIMEOUT = httpx.Timeout(45.0, connect=10.0)


class ChatGptError(RuntimeError):
    """A classified ChatGPT auth/quota failure.

    ``code`` is stable and safe to return to the UI. Messages may contain
    upstream text, but callers must not forward token material.
    """

    def __init__(self, code: str, message: str = ""):
        super().__init__(message or code)
        self.code = code


@dataclass
class ChatGptCredentials:
    access_token: str = ""
    refresh_token: str = ""
    id_token: str = ""
    account_id: str = ""
    email: str = ""
    plan_type: str = ""
    expires_at: float = 0.0
    last_refresh: str = ""

    @classmethod
    def from_json(cls, raw: str | None) -> "ChatGptCredentials":
        if not raw:
            return cls()
        try:
            body = json.loads(raw)
        except (TypeError, ValueError):
            # A legacy API key pasted into a ChatGPT row is not usable. Treat it
            # as missing rather than leaking the value into an error.
            return cls()
        if not isinstance(body, dict):
            return cls()
        fields = {name: body.get(name) for name in cls.__dataclass_fields__}
        creds = cls(
            access_token=str(fields.get("access_token") or ""),
            refresh_token=str(fields.get("refresh_token") or ""),
            id_token=str(fields.get("id_token") or ""),
            account_id=str(fields.get("account_id") or ""),
            email=str(fields.get("email") or ""),
            plan_type=str(fields.get("plan_type") or ""),
            expires_at=float(fields.get("expires_at") or 0),
            last_refresh=str(fields.get("last_refresh") or ""),
        )
        if not creds.account_id and creds.id_token:
            claims = parse_id_token(creds.id_token)
            creds.account_id = claims.get("account_id", "")
            creds.email = creds.email or claims.get("email", "")
            creds.plan_type = creds.plan_type or claims.get("plan_type", "")
            creds.expires_at = creds.expires_at or claims.get("expires_at", 0.0)
        return creds

    def to_json(self) -> str:
        return json.dumps(asdict(self), separators=(",", ":"))

    def needs_refresh(self, now: float | None = None) -> bool:
        if not self.access_token:
            return True
        current = now if now is not None else time.time()
        # A missing expiry is treated as stale: refreshing is cheap and safe.
        return not self.expires_at or self.expires_at - REFRESH_SKEW_SECONDS <= current

    def public_account(self) -> dict:
        return {
            "account_id": self.account_id,
            "email": self.email,
            "plan_type": self.plan_type,
        }


def _originator() -> str:
    import os

    return (os.environ.get(ORIGINATOR_ENV) or ORIGINATOR).strip() or ORIGINATOR


def _jwt_payload(jwt: str) -> dict:
    parts = (jwt or "").split(".")
    if len(parts) != 3 or not parts[1]:
        return {}
    payload = parts[1] + "=" * (-len(parts[1]) % 4)
    try:
        value = json.loads(base64.urlsafe_b64decode(payload.encode()).decode())
    except (ValueError, UnicodeDecodeError, json.JSONDecodeError):
        return {}
    return value if isinstance(value, dict) else {}


def parse_id_token(id_token: str) -> dict:
    """Extract the account identity Codex puts under the OpenAI auth claim.

    Signature verification is deliberately not done: the token is only used to
    label the local account and to derive the header the API itself verifies.
    """
    payload = _jwt_payload(id_token)
    auth = payload.get("https://api.openai.com/auth")
    auth = auth if isinstance(auth, dict) else {}
    profile = payload.get("profile")
    profile = profile if isinstance(profile, dict) else {}
    exp = payload.get("exp")
    try:
        expires_at = float(exp) if exp is not None else 0.0
    except (TypeError, ValueError):
        expires_at = 0.0
    return {
        "account_id": str(auth.get("chatgpt_account_id") or ""),
        "plan_type": str(auth.get("chatgpt_plan_type") or ""),
        "email": str(payload.get("email") or profile.get("email") or ""),
        "expires_at": expires_at,
    }


def _credentials_from_tokens(tokens: dict) -> ChatGptCredentials:
    id_token = str(tokens.get("id_token") or "")
    claims = parse_id_token(id_token)
    expires_at = claims.get("expires_at") or 0.0
    if not expires_at:
        try:
            expires_in = float(tokens.get("expires_in") or 0)
        except (TypeError, ValueError):
            expires_in = 0
        expires_at = time.time() + expires_in if expires_in else time.time() + 3600
    return ChatGptCredentials(
        access_token=str(tokens.get("access_token") or ""),
        refresh_token=str(tokens.get("refresh_token") or ""),
        id_token=id_token,
        account_id=claims.get("account_id", ""),
        email=claims.get("email", ""),
        plan_type=claims.get("plan_type", ""),
        expires_at=float(expires_at),
        last_refresh=datetime.now(UTC).isoformat(),
    )


async def exchange_authorization_code(code: str, code_verifier: str, redirect_uri: str) -> ChatGptCredentials:
    body = {
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "code": code,
        "redirect_uri": redirect_uri,
        "code_verifier": code_verifier,
    }
    async with httpx.AsyncClient(timeout=_HTTP_TIMEOUT) as client:
        response = await client.post(f"{AUTH_ISSUER}/oauth/token", data=body)
    if not response.is_success:
        raise ChatGptError("chatgpt_login_failed", f"token exchange returned {response.status_code}")
    try:
        tokens = response.json()
    except ValueError as exc:
        raise ChatGptError("chatgpt_login_failed", "token exchange returned invalid JSON") from exc
    return _credentials_from_tokens(tokens if isinstance(tokens, dict) else {})


async def refresh_credentials(creds: ChatGptCredentials) -> ChatGptCredentials:
    if not creds.refresh_token:
        raise ChatGptError(AUTH_REQUIRED, "no refresh token")
    body = {
        "grant_type": "refresh_token",
        "client_id": CLIENT_ID,
        "refresh_token": creds.refresh_token,
        "refresh_reason": "proactive_expiry",
    }
    async with httpx.AsyncClient(timeout=_HTTP_TIMEOUT) as client:
        response = await client.post(f"{AUTH_ISSUER}/oauth/token", json=body)
    if not response.is_success:
        try:
            detail = response.json()
        except ValueError:
            detail = {}
        code = ""
        if isinstance(detail, dict):
            code = str(detail.get("error") or detail.get("code") or "")
        if response.status_code == 400 and code in {
            "invalid_grant",
            "refresh_token_expired",
            "refresh_token_reused",
            "refresh_token_invalidated",
        }:
            raise ChatGptError(AUTH_EXPIRED, "refresh token is no longer valid")
        if response.status_code in {401, 403}:
            raise ChatGptError(AUTH_EXPIRED, f"refresh rejected with {response.status_code}")
        raise ChatGptError("chatgpt_refresh_failed", f"refresh returned {response.status_code}")
    try:
        tokens = response.json()
    except ValueError as exc:
        raise ChatGptError("chatgpt_refresh_failed", "refresh returned invalid JSON") from exc
    refreshed = _credentials_from_tokens(tokens if isinstance(tokens, dict) else {})
    # A refresh response may omit the refresh token or ID token; keep the old
    # values so the next refresh and account label keep working.
    refreshed.refresh_token = refreshed.refresh_token or creds.refresh_token
    refreshed.id_token = refreshed.id_token or creds.id_token
    refreshed.account_id = refreshed.account_id or creds.account_id
    refreshed.email = refreshed.email or creds.email
    refreshed.plan_type = refreshed.plan_type or creds.plan_type
    return refreshed


async def revoke_credentials(creds: ChatGptCredentials) -> None:
    token = creds.refresh_token or creds.access_token
    if not token:
        return
    body = {
        "token": token,
        "token_type_hint": "refresh_token" if creds.refresh_token else "access_token",
    }
    if creds.refresh_token:
        body["client_id"] = CLIENT_ID
    try:
        async with httpx.AsyncClient(timeout=httpx.Timeout(10.0, connect=5.0)) as client:
            await client.post(f"{AUTH_ISSUER}/oauth/revoke", json=body)
    except httpx.HTTPError:
        # Logout must remove local credentials even when the network is down.
        pass


def auth_headers(creds: ChatGptCredentials, session_id: str | None = None) -> dict:
    headers = {
        "Authorization": f"Bearer {creds.access_token}",
        "chatgpt-account-id": creds.account_id,
        "originator": _originator(),
        "User-Agent": _user_agent(),
    }
    if session_id:
        headers["session-id"] = session_id
        headers["thread-id"] = session_id
    return headers


def _user_agent() -> str:
    try:
        from importlib.metadata import version

        return f"mix-agent/{version('mix-agent')}"
    except Exception:  # noqa: BLE001 - metadata is optional on source checkouts
        return "mix-agent/0"


# ---------------------------------------------------------------------------
# Device-code login
# ---------------------------------------------------------------------------

_DEVICE_LOGINS: dict[str, dict] = {}


def _purge_expired_logins(now: float) -> None:
    for key in [k for k, v in _DEVICE_LOGINS.items() if v.get("expires_at", 0) <= now]:
        _DEVICE_LOGINS.pop(key, None)


async def start_device_login(provider_id: str, owner_id: str) -> dict:
    async with httpx.AsyncClient(timeout=_HTTP_TIMEOUT) as client:
        response = await client.post(
            f"{AUTH_ISSUER}/api/accounts/deviceauth/usercode",
            json={"client_id": CLIENT_ID},
            headers={"originator": _originator()},
        )
    if response.status_code == 404:
        raise ChatGptError(
            "chatgpt_device_unavailable",
            "device code login is not enabled for this account",
        )
    if not response.is_success:
        raise ChatGptError(
            "chatgpt_device_failed",
            f"device code request returned {response.status_code}",
        )
    try:
        payload = response.json()
    except ValueError as exc:
        raise ChatGptError("chatgpt_device_failed", "device code returned invalid JSON") from exc
    if not isinstance(payload, dict):
        raise ChatGptError("chatgpt_device_failed", "device code returned invalid JSON")
    try:
        interval = int(float(payload.get("interval") or 5))
    except (TypeError, ValueError):
        interval = 5
    now = time.time()
    login_id = uuid.uuid4().hex
    _purge_expired_logins(now)
    _DEVICE_LOGINS[login_id] = {
        "provider_id": provider_id,
        "owner_id": owner_id,
        "device_auth_id": str(payload.get("device_auth_id") or ""),
        "user_code": str(payload.get("user_code") or payload.get("usercode") or ""),
        "interval": max(interval, LOGIN_POLL_FLOOR_SECONDS),
        "created_at": now,
        "expires_at": now + DEVICE_TTL_SECONDS,
        "next_poll_at": now,
        "status": "pending",
    }
    return {
        "login_id": login_id,
        "verification_url": DEVICE_VERIFICATION_URL,
        "user_code": _DEVICE_LOGINS[login_id]["user_code"],
        "expires_in": DEVICE_TTL_SECONDS,
        "interval": _DEVICE_LOGINS[login_id]["interval"],
    }


def device_login_state(login_id: str, owner_id: str) -> dict | None:
    state = _DEVICE_LOGINS.get(login_id)
    if not state or state.get("owner_id") != owner_id:
        return None
    if state.get("expires_at", 0) <= time.time():
        state["status"] = "expired"
        state["error"] = "chatgpt_login_expired"
    return state


def cancel_device_login(login_id: str, owner_id: str) -> bool:
    state = _DEVICE_LOGINS.get(login_id)
    if not state or state.get("owner_id") != owner_id:
        return False
    _DEVICE_LOGINS.pop(login_id, None)
    return True


async def poll_device_login(login_id: str, owner_id: str) -> dict:
    state = device_login_state(login_id, owner_id)
    if not state:
        raise ChatGptError("chatgpt_login_not_found", "login flow not found")
    if state.get("status") in {"complete", "error", "expired"}:
        return _device_public(state)

    now = time.time()
    if now < state.get("next_poll_at", 0):
        return _device_public(state)

    async with httpx.AsyncClient(timeout=_HTTP_TIMEOUT) as client:
        response = await client.post(
            f"{AUTH_ISSUER}/api/accounts/deviceauth/token",
            json={
                "device_auth_id": state.get("device_auth_id", ""),
                "user_code": state.get("user_code", ""),
            },
            headers={"originator": _originator()},
        )

    if response.status_code in {403, 404}:
        state["next_poll_at"] = now + state.get("interval", 5)
        return _device_public(state)
    if not response.is_success:
        state["status"] = "error"
        state["error"] = f"chatgpt_device_failed:{response.status_code}"
        return _device_public(state)
    try:
        code_body = response.json()
    except ValueError:
        state["status"] = "error"
        state["error"] = "chatgpt_device_failed"
        return _device_public(state)
    if not isinstance(code_body, dict):
        state["status"] = "error"
        state["error"] = "chatgpt_device_failed"
        return _device_public(state)
    try:
        creds = await exchange_authorization_code(
            str(code_body.get("authorization_code") or ""),
            str(code_body.get("code_verifier") or ""),
            DEVICE_REDIRECT_URI,
        )
    except ChatGptError as exc:
        state["status"] = "error"
        state["error"] = exc.code
        return _device_public(state)
    state["status"] = "complete"
    state["credentials"] = creds
    state["account"] = creds.public_account()
    return _device_public(state)


def _device_public(state: dict) -> dict:
    out = {
        "status": state.get("status", "pending"),
        "error": state.get("error"),
        "account": state.get("account"),
    }
    # Internal field for the route to persist; never exposed (response_model
    # filters it out, and it is not JSON-serialisable as an API object).
    if state.get("status") == "complete" and isinstance(state.get("credentials"), ChatGptCredentials):
        out["credentials"] = state["credentials"]
    return out


def save_credentials(db, provider, creds: ChatGptCredentials, owner_id: str) -> None:
    """Persist tokens into the provider's encrypted secret and metadata."""
    raw = creds.to_json()
    secret_id = provider.data.get("secret_id")
    secret = db.get(Secret, secret_id) if secret_id else None
    if secret is not None:
        secret.data = encrypt(raw, secret.data.get("purpose", "provider"))
    else:
        secret_id = store_secret(db, owner_id, raw, "provider")
    provider.data = {
        **provider.data,
        "secret_id": secret_id,
        "status": "ready",
        "auth": {
            "mode": "chatgpt_oauth",
            **creds.public_account(),
            "last_refresh": creds.last_refresh,
        },
        "billing_mode": "subscription",
    }
    _QUOTA_CACHE.pop(creds.account_id or secret_id, None)


def clear_credentials(db, provider) -> None:
    secret_id = provider.data.get("secret_id")
    auth = provider.data.get("auth") if isinstance(provider.data.get("auth"), dict) else {}
    if secret_id:
        secret = db.get(Secret, secret_id)
        if secret is not None:
            db.delete(secret)
    provider.data = {key: value for key, value in provider.data.items() if key not in {"secret_id", "auth"}}
    provider.data["status"] = "needs_login"
    _QUOTA_CACHE.pop((auth or {}).get("account_id") or secret_id, None)


# ---------------------------------------------------------------------------
# Quota
# ---------------------------------------------------------------------------

_QUOTA_CACHE: dict[str, tuple[float, dict]] = {}


def normalize_usage(payload: dict, captured_at: float | None = None) -> dict:
    now = captured_at if captured_at is not None else time.time()
    payload = payload if isinstance(payload, dict) else {}
    rate_limit = payload.get("rate_limit")
    rate_limit = rate_limit if isinstance(rate_limit, dict) else {}
    windows = []
    for key in ("primary_window", "secondary_window"):
        window = rate_limit.get(key)
        if not isinstance(window, dict):
            continue
        seconds = _as_int(window.get("limit_window_seconds"))
        if not seconds:
            seconds = _as_int(window.get("window_minutes")) * 60
        reset_at = _as_int(window.get("reset_at"))
        if not reset_at:
            reset_at = int(now + (_as_int(window.get("reset_after_seconds")) or 0))
        used = _as_float(window.get("used_percent"))
        windows.append(
            {
                "id": _window_id(seconds, key),
                "used_percent": max(0.0, min(100.0, used)),
                "window_minutes": int(round(seconds / 60)) if seconds else 0,
                "window_seconds": seconds,
                "resets_at": reset_at,
            }
        )
    credits = payload.get("credits") if isinstance(payload, dict) else None
    if not isinstance(credits, dict):
        credits = None
    return {
        "plan_type": str(payload.get("plan_type") or ""),
        "captured_at": datetime.fromtimestamp(now, UTC).isoformat(),
        "windows": windows,
        "credits": credits,
        "limit_reached": bool(rate_limit.get("limit_reached") or rate_limit.get("allowed") is False),
    }


def quota_from_headers(headers, captured_at: float | None = None) -> dict | None:
    """Parse the rate-limit headers Codex reads from an inference response."""
    now = captured_at if captured_at is not None else time.time()
    windows = []
    for suffix, key in (("primary", "primary_window"), ("secondary", "secondary_window")):
        used_raw = headers.get(f"x-codex-{suffix}-used-percent")
        if used_raw is None:
            continue
        minutes = _as_int(headers.get(f"x-codex-{suffix}-window-minutes"))
        reset_at = _as_int(headers.get(f"x-codex-{suffix}-reset-at"))
        windows.append(
            {
                "id": _window_id(minutes * 60 if minutes else 0, key),
                "used_percent": max(0.0, min(100.0, _as_float(used_raw))),
                "window_minutes": minutes or 0,
                "window_seconds": (minutes or 0) * 60,
                "resets_at": reset_at or int(now),
            }
        )
    if not windows:
        return None
    has_credits = headers.get("x-codex-credits-has-credits")
    credits = None
    if has_credits is not None:
        credits = {
            "has_credits": str(has_credits).lower() in {"1", "true"},
            "unlimited": str(headers.get("x-codex-credits-unlimited", "")).lower() in {"1", "true"},
            "balance": headers.get("x-codex-credits-balance") or "",
        }
    return {
        "plan_type": "",
        "captured_at": datetime.fromtimestamp(now, UTC).isoformat(),
        "windows": windows,
        "credits": credits,
        "limit_reached": str(headers.get("x-codex-rate-limit-reached-type") or "") not in {"", "None"},
    }


def _window_id(seconds: int, fallback: str) -> str:
    if not seconds:
        return fallback
    if seconds <= 6 * 3600:
        return "5h"
    if seconds >= 6 * 86400:
        return "week"
    return f"{max(1, round(seconds / 3600))}h"


def _as_int(value) -> int:
    try:
        return int(float(value))
    except (TypeError, ValueError):
        return 0


def _as_float(value) -> float:
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


async def fetch_usage(creds: ChatGptCredentials) -> dict:
    async with httpx.AsyncClient(timeout=httpx.Timeout(20.0, connect=10.0)) as client:
        response = await client.get(
            CHATGPT_USAGE_URL,
            headers=auth_headers(creds),
        )
    if response.status_code in {401, 403}:
        raise ChatGptError(AUTH_EXPIRED, "usage endpoint rejected the token")
    if not response.is_success:
        raise ChatGptError("chatgpt_quota_failed", f"usage returned {response.status_code}")
    try:
        payload = response.json()
    except ValueError as exc:
        raise ChatGptError("chatgpt_quota_failed", "usage returned invalid JSON") from exc
    return normalize_usage(payload if isinstance(payload, dict) else {})


# ---------------------------------------------------------------------------
# Auth manager
# ---------------------------------------------------------------------------

_REFRESH_LOCKS: dict[str, asyncio.Lock] = {}


class ChatGptAuth:
    """Refreshable credentials for one provider row.

    The manager is constructed per Adapter, but the refresh lock and quota
    cache are process-wide so concurrent runs cannot replay one refresh token.
    """

    def __init__(self, secret_id: str | None, raw: str, provider_id: str | None = None):
        self.secret_id = secret_id or ""
        self.provider_id = provider_id or ""
        self.creds = ChatGptCredentials.from_json(raw)

    @property
    def connected(self) -> bool:
        return bool(self.creds.access_token or self.creds.refresh_token)

    def _lock(self) -> asyncio.Lock:
        key = self.secret_id or self.creds.account_id or self.provider_id or "default"
        return _REFRESH_LOCKS.setdefault(key, asyncio.Lock())

    def _load_persisted(self) -> ChatGptCredentials | None:
        if not self.secret_id:
            return None
        from mix_agent.auth.security import read_secret
        from mix_agent.db.session import SessionLocal

        try:
            with SessionLocal() as db:
                raw = read_secret(db, self.secret_id)
        except Exception:  # noqa: BLE001 - a DB hiccup must not break the run
            return None
        persisted = ChatGptCredentials.from_json(raw)
        return persisted if persisted.access_token else None

    def _persist(self) -> None:
        if not self.secret_id:
            return
        from mix_agent.db.session import SessionLocal

        try:
            with SessionLocal() as db:
                secret = db.get(Secret, self.secret_id)
                if secret is None:
                    return
                secret.data = encrypt(self.creds.to_json(), secret.data.get("purpose", "provider"))
                db.commit()
        except Exception:  # noqa: BLE001 - persistence is retried on the next run
            return

    async def token(self, force_refresh: bool = False) -> str:
        if not self.connected:
            raise ChatGptError(AUTH_REQUIRED, "ChatGPT login is required")
        if force_refresh or self.creds.needs_refresh():
            await self.refresh()
        return self.creds.access_token

    async def refresh(self) -> None:
        async with self._lock():
            # Another Adapter for the same provider may have refreshed already.
            persisted = self._load_persisted()
            if (
                persisted is not None
                and persisted.access_token != self.creds.access_token
                and not persisted.needs_refresh()
            ):
                self.creds = persisted
                return
            self.creds = await refresh_credentials(self.creds)
            self._persist()

    def headers(self, session_id: str | None = None) -> dict:
        return auth_headers(self.creds, session_id)

    def observe_headers(self, response) -> None:
        headers = getattr(response, "headers", None)
        if headers is None:
            return
        quota = quota_from_headers(headers)
        if quota:
            key = self.creds.account_id or self.secret_id
            _QUOTA_CACHE[key] = (time.time(), quota)

    def observe_rate_limit(self, event) -> None:
        """Cache the `codex.rate_limits` SSE event without trusting its shape."""
        try:
            if isinstance(event, dict):
                payload = event
            else:
                payload = event.model_dump(mode="json") if hasattr(event, "model_dump") else {}
        except Exception:  # noqa: BLE001 - a malformed event must not fail the turn
            return
        if not isinstance(payload, dict):
            return
        details = payload.get("rate_limits")
        details = details if isinstance(details, dict) else {}
        quota = normalize_usage(
            {
                "plan_type": payload.get("plan_type") or self.creds.plan_type,
                "rate_limit": {
                    "primary_window": details.get("primary"),
                    "secondary_window": details.get("secondary"),
                    "limit_reached": details.get("limit_reached"),
                },
                "credits": payload.get("credits"),
            }
        )
        if quota.get("windows"):
            key = self.creds.account_id or self.secret_id
            _QUOTA_CACHE[key] = (time.time(), quota)

    async def quota(self, force: bool = False) -> dict:
        key = self.creds.account_id or self.secret_id
        cached = _QUOTA_CACHE.get(key)
        if cached and not force and time.time() - cached[0] < QUOTA_TTL_SECONDS:
            return cached[1]
        if self.creds.needs_refresh():
            await self.refresh()
        try:
            quota = await fetch_usage(self.creds)
        except ChatGptError as exc:
            if exc.code == AUTH_EXPIRED and self.creds.refresh_token:
                await self.refresh()
                quota = await fetch_usage(self.creds)
            else:
                raise
        _QUOTA_CACHE[key] = (time.time(), quota)
        return quota

    async def logout(self) -> None:
        await revoke_credentials(self.creds)
        key = self.creds.account_id or self.secret_id
        _QUOTA_CACHE.pop(key, None)
        self.creds = ChatGptCredentials()
