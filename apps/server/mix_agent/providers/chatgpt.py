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
import logging
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

logger = logging.getLogger(__name__)


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
    # Set when the refresh endpoint rejected the stored refresh token. The
    # access token may still work until it expires, but the UI stops showing
    # the account as fully connected and offers "sign in again".
    auth_failed: bool = False

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
            auth_failed=bool(fields.get("auth_failed") or False),
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

    def connected(self) -> bool:
        return bool(self.access_token or self.refresh_token)

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
        try:
            response = await client.post(f"{AUTH_ISSUER}/oauth/token", data=body)
        except httpx.HTTPError as exc:
            raise ChatGptError(
                "chatgpt_login_failed", f"token exchange failed: {type(exc).__name__}"
            ) from exc
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
        try:
            response = await client.post(f"{AUTH_ISSUER}/oauth/token", json=body)
        except httpx.HTTPError as exc:
            raise ChatGptError(
                "chatgpt_refresh_failed", f"token refresh failed: {type(exc).__name__}"
            ) from exc
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
    try:
        async with httpx.AsyncClient(timeout=_HTTP_TIMEOUT) as client:
            response = await client.post(
                f"{AUTH_ISSUER}/api/accounts/deviceauth/usercode",
                json={"client_id": CLIENT_ID},
                headers={"originator": _originator()},
            )
    except httpx.HTTPError as exc:
        raise ChatGptError(
            "chatgpt_device_failed", f"device code request failed: {type(exc).__name__}"
        ) from exc
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
        "poll_failures": 0,
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
    # Reserve this poll before awaiting: two concurrent status requests would
    # otherwise both pass the check and hit the auth endpoint twice.
    state["next_poll_at"] = now + max(1, int(state.get("interval", 5)))

    try:
        async with httpx.AsyncClient(timeout=_HTTP_TIMEOUT) as client:
            response = await client.post(
                f"{AUTH_ISSUER}/api/accounts/deviceauth/token",
                json={
                    "device_auth_id": state.get("device_auth_id", ""),
                    "user_code": state.get("user_code", ""),
                },
                headers={"originator": _originator()},
            )
    except httpx.HTTPError:
        # A temporary network failure must not invalidate a code the user is
        # still typing. Keep the flow alive and back off until the TTL.
        _retry_later(state, now)
        return _device_public(state)

    if response.status_code in {403, 404}:
        body = response.text.lower()
        if any(marker in body for marker in ("expired", "invalid_grant", "device_code_expired")):
            state["status"] = "error"
            state["error"] = "chatgpt_login_expired"
            return _device_public(state)
        _poll_again(state, now)
        return _device_public(state)
    if not response.is_success:
        # 408/425/429 are transient just like 5xx; killing the flow here made a
        # momentary rate limit look like an invalid code.
        if response.status_code >= 500 or response.status_code in {408, 425, 429}:
            _retry_later(state, now)
            return _device_public(state)
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
    except httpx.HTTPError:
        _retry_later(state, now)
        return _device_public(state)
    except ChatGptError as exc:
        state["status"] = "error"
        state["error"] = exc.code
        return _device_public(state)
    state["status"] = "complete"
    state["credentials"] = creds
    state["account"] = creds.public_account()
    return _device_public(state)


def _poll_again(state: dict, now: float) -> None:
    """The user has not authorized yet: wait one normal interval."""
    state["poll_failures"] = 0
    state["next_poll_at"] = now + max(1, int(state.get("interval", 5)))


def _retry_later(state: dict, now: float) -> None:
    """Schedule the next poll with bounded exponential backoff."""
    failures = int(state.get("poll_failures", 0)) + 1
    state["poll_failures"] = failures
    base = max(1, int(state.get("interval", 5)))
    delay = min(60, base * (2 ** min(failures - 1, 4)))
    state["next_poll_at"] = now + delay


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
    _drop_quota_cache(creds.account_id, secret_id, getattr(provider, "id", None))


def clear_credentials(db, provider) -> None:
    secret_id = provider.data.get("secret_id")
    auth = provider.data.get("auth") if isinstance(provider.data.get("auth"), dict) else {}
    if secret_id:
        secret = db.get(Secret, secret_id)
        if secret is not None:
            db.delete(secret)
    provider.data = {key: value for key, value in provider.data.items() if key not in {"secret_id", "auth"}}
    provider.data["status"] = "needs_login"
    _drop_quota_cache((auth or {}).get("account_id"), secret_id, getattr(provider, "id", None))


# ---------------------------------------------------------------------------
# Quota
# ---------------------------------------------------------------------------

_QUOTA_CACHE: dict[str, tuple[float, dict]] = {}
# Keep a last-known snapshot around while the UI might still be open, but do
# not let a long-running server accumulate one entry per account forever.
_QUOTA_CACHE_MAX_AGE = 3600.0


def _quota_cache_key(*candidates: str | None) -> str:
    for candidate in candidates:
        if candidate:
            return str(candidate)
    return "default"


def _drop_quota_cache(*keys: str | None) -> None:
    for key in keys:
        if key:
            _QUOTA_CACHE.pop(str(key), None)


def _read_quota_cache(key: str, now: float | None = None) -> tuple[float, dict] | None:
    now = time.time() if now is None else now
    cached = _QUOTA_CACHE.get(key)
    if cached is None:
        return None
    if now - cached[0] > _QUOTA_CACHE_MAX_AGE:
        _QUOTA_CACHE.pop(key, None)
        return None
    return cached


def _write_quota_cache(key: str, quota: dict) -> None:
    now = time.time()
    for stale in [k for k, (at, _) in _QUOTA_CACHE.items() if now - at > _QUOTA_CACHE_MAX_AGE]:
        _QUOTA_CACHE.pop(stale, None)
    _QUOTA_CACHE[key] = (now, quota)


def normalize_usage(payload: dict, captured_at: float | None = None) -> dict:
    now = captured_at if captured_at is not None else time.time()
    payload = payload if isinstance(payload, dict) else {}
    rate_limit = payload.get("rate_limit")
    rate_limit = rate_limit if isinstance(rate_limit, dict) else {}
    windows = []
    for key, fallback in (("primary_window", "5h"), ("secondary_window", "week")):
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
                "id": _window_id(seconds, fallback),
                "used_percent": max(0.0, min(100.0, used)),
                "window_minutes": (seconds + 30) // 60 if seconds else 0,
                "window_seconds": seconds,
                "resets_at": reset_at,
            }
        )
    credits = _normalize_credits(payload.get("credits"))
    return {
        "plan_type": str(payload.get("plan_type") or ""),
        "captured_at": datetime.fromtimestamp(now, UTC).isoformat(),
        "windows": windows,
        "credits": credits,
        "limit_reached": _as_bool(rate_limit.get("limit_reached")) is True
        or (rate_limit.get("allowed") is not None and _as_bool(rate_limit.get("allowed")) is False),
    }


def quota_from_headers(headers, captured_at: float | None = None) -> dict | None:
    """Parse the rate-limit headers Codex reads from an inference response."""
    now = captured_at if captured_at is not None else time.time()
    windows = []
    for suffix, fallback in (("primary", "5h"), ("secondary", "week")):
        used_raw = headers.get(f"x-codex-{suffix}-used-percent")
        if used_raw is None:
            continue
        minutes = _as_int(headers.get(f"x-codex-{suffix}-window-minutes"))
        reset_at = _as_int(headers.get(f"x-codex-{suffix}-reset-at"))
        windows.append(
            {
                "id": _window_id(minutes * 60 if minutes else 0, fallback),
                "used_percent": max(0.0, min(100.0, _as_float(used_raw))),
                "window_minutes": minutes or 0,
                "window_seconds": (minutes or 0) * 60,
                "resets_at": reset_at,
            }
        )
    if not windows:
        return None
    has_credits = headers.get("x-codex-credits-has-credits")
    credits = None
    if has_credits is not None:
        credits = {
            "has_credits": _as_bool(has_credits) is True,
            "unlimited": _as_bool(headers.get("x-codex-credits-unlimited")) is True,
            "balance": str(headers.get("x-codex-credits-balance") or ""),
        }
    return {
        "plan_type": "",
        "captured_at": datetime.fromtimestamp(now, UTC).isoformat(),
        "windows": windows,
        "credits": credits,
        "limit_reached": str(headers.get("x-codex-rate-limit-reached-type") or "").strip().lower()
        not in {"", "none", "false", "0"},
    }


def _window_id(seconds: int, fallback: str) -> str:
    if not seconds:
        return fallback
    # Exactly six hours was labelled as the five-hour window, so a plan with a
    # 6h primary window rendered the wrong name.
    if seconds < 6 * 3600:
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


def _as_bool(value) -> bool | None:
    """Normalize the loose booleans the private backend emits.

    The same field has been observed as ``true``, ``1`` and ``"true"``. The old
    ``is True`` / ``is False`` checks silently missed the non-bool shapes.
    """
    if isinstance(value, bool):
        return value
    if isinstance(value, (int, float)):
        return value != 0
    if isinstance(value, str):
        lowered = value.strip().lower()
        if lowered in {"1", "true", "yes", "on"}:
            return True
        if lowered in {"0", "false", "no", "off", ""}:
            return False
    return None


def _normalize_credits(raw) -> dict | None:
    """Return the fixed credits shape the API schema expects.

    The private backend emits ``balance`` as a number, a string, or null. The
    response model declares ``str``, so forwarding the raw value made a valid
    usage payload fail validation with a 500. Normalize here, matching the Rust
    edition.
    """
    if not isinstance(raw, dict):
        return None
    balance = raw.get("balance")
    return {
        "has_credits": _as_bool(raw.get("has_credits")) is True,
        "unlimited": _as_bool(raw.get("unlimited")) is True,
        "balance": "" if balance is None else str(balance),
    }


async def fetch_usage(creds: ChatGptCredentials) -> dict:
    try:
        async with httpx.AsyncClient(timeout=httpx.Timeout(20.0, connect=10.0)) as client:
            response = await client.get(
                CHATGPT_USAGE_URL,
                headers=auth_headers(creds),
            )
    except httpx.HTTPError as exc:
        # Keep the type stable so callers can fall back to the last snapshot
        # instead of losing it to a timeout or a transient connection failure.
        raise ChatGptError("chatgpt_quota_failed", f"usage request failed: {type(exc).__name__}") from exc
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

    async def _persist(self) -> bool:
        if not self.secret_id:
            return True
        from mix_agent.db.session import SessionLocal

        for attempt in range(3):
            try:
                with SessionLocal() as db:
                    secret = db.get(Secret, self.secret_id)
                    if secret is None:
                        return True
                    secret.data = encrypt(self.creds.to_json(), secret.data.get("purpose", "provider"))
                    db.commit()
                return True
            except Exception as err:  # noqa: BLE001 - retry, then surface to the caller
                logger.error(
                    "could not persist refreshed ChatGPT credentials (attempt %s): %s",
                    attempt + 1,
                    type(err).__name__,
                )
                await asyncio.sleep(0.05 * (attempt + 1))
        return False

    def _update_provider_status(self, status: str) -> None:
        if not self.provider_id:
            return
        from mix_agent.db.models import Provider
        from mix_agent.db.session import SessionLocal

        try:
            with SessionLocal() as db:
                provider = db.get(Provider, self.provider_id)
                if provider is None:
                    return
                provider.data = {**provider.data, "status": status}
                db.commit()
        except Exception as err:  # noqa: BLE001 - metadata must not break inference
            logger.warning(
                "could not update ChatGPT provider status (%s): %s",
                status,
                type(err).__name__,
            )

    async def token(self, force_refresh: bool = False) -> str:
        if not self.connected:
            raise ChatGptError(AUTH_REQUIRED, "ChatGPT login is required")
        needs_refresh = self.creds.needs_refresh()
        # `auth_failed` means the refresh endpoint rejected our stored refresh
        # token. Retrying it on every turn only hammers the auth endpoint; a
        # still-valid access token remains usable, so fail only when a refresh
        # would actually be required. The post-401 path passes `force_refresh`
        # because the server just proved the current access token unusable.
        if self.creds.auth_failed and needs_refresh and not force_refresh:
            raise ChatGptError(AUTH_REQUIRED, "ChatGPT login must be renewed")
        if force_refresh or needs_refresh:
            await self.refresh(force=force_refresh)
        return self.creds.access_token

    async def refresh(self, force: bool = False) -> None:
        async with self._lock():
            # Another Adapter for the same provider may have refreshed already.
            # Adopt it even when it is already close to expiry: OpenAI rotates
            # refresh tokens, so refreshing with our previous token instead
            # would be rejected (`invalid_grant`) and log the user out. This is
            # the same order the Rust edition uses.
            persisted = self._load_persisted()
            if persisted is not None and persisted.connected():
                if (
                    persisted.access_token != self.creds.access_token
                    or persisted.refresh_token != self.creds.refresh_token
                ):
                    self.creds = persisted
                    if not persisted.needs_refresh():
                        return
            if self.creds.auth_failed and not force:
                raise ChatGptError(AUTH_REQUIRED, "ChatGPT login must be renewed")
            was_failed = self.creds.auth_failed
            try:
                self.creds = await refresh_credentials(self.creds)
            except ChatGptError as exc:
                if exc.code in {AUTH_REQUIRED, AUTH_EXPIRED}:
                    self.creds.auth_failed = True
                    await self._persist()
                    self._update_provider_status("needs_login")
                raise
            if not await self._persist():
                raise ChatGptError(
                    "chatgpt_credentials_save_failed",
                    "the refreshed ChatGPT credentials could not be saved",
                )
            if was_failed:
                self._update_provider_status("ready")

    def headers(self, session_id: str | None = None) -> dict:
        return auth_headers(self.creds, session_id)

    def _quota_key(self) -> str:
        # Provider id is stable across token refresh; account/secret are only
        # fallbacks for managers built without one.
        return _quota_cache_key(self.provider_id, self.creds.account_id, self.secret_id, "default")

    def observe_headers(self, response) -> None:
        headers = getattr(response, "headers", None)
        if headers is None:
            return
        quota = quota_from_headers(headers)
        if quota:
            _write_quota_cache(self._quota_key(), quota)

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
            _write_quota_cache(self._quota_key(), quota)

    async def quota(self, force: bool = False) -> dict:
        cached = _read_quota_cache(self._quota_key())
        if cached and not force and time.time() - cached[0] < QUOTA_TTL_SECONDS:
            return cached[1]

        def fallback(error: ChatGptError) -> dict:
            # A transient failure must not hide the last snapshot the inference
            # headers / SSE events produced. Auth failures still need a fresh
            # login, so surface those.
            if error.code in {AUTH_REQUIRED, AUTH_EXPIRED}:
                raise error
            if cached:
                return cached[1]
            raise error

        try:
            # Proactive refresh belongs inside the fallback: a network failure
            # here used to propagate before the last snapshot was considered.
            # Remember whether one was already attempted so a rejected token
            # is not refreshed twice in a row.
            refreshed_first = self.creds.needs_refresh()
            if refreshed_first:
                await self.refresh()
            quota = await fetch_usage(self.creds)
        except ChatGptError as exc:
            if exc.code == AUTH_EXPIRED and self.creds.refresh_token and not refreshed_first:
                try:
                    await self.refresh(force=True)
                    quota = await fetch_usage(self.creds)
                except ChatGptError as retry_exc:
                    return fallback(retry_exc)
            else:
                return fallback(exc)
        _write_quota_cache(self._quota_key(), quota)
        return quota

    async def logout(self) -> None:
        await revoke_credentials(self.creds)
        _drop_quota_cache(self._quota_key(), self.provider_id, self.creds.account_id, self.secret_id)
        self.creds = ChatGptCredentials()
