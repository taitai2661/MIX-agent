import asyncio
import base64
import hashlib
import secrets
import time as _time
from collections import defaultdict, deque
from datetime import UTC, timedelta

from argon2 import PasswordHasher
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from fastapi import HTTPException, Request

from mix_agent import config
from mix_agent.db.models import Secret, Session, now

passwords = PasswordHasher()

_master_key: bytes | None = None

# Uppercase: bounded brute-force protection for auth surfaces.  Failures only;
# successful operations never consume or write to these in-memory buckets.
LOGIN_WINDOW_SECONDS = 300
LOGIN_ACCOUNT_LIMIT = 10
LOGIN_IP_LIMIT = 20
ACCOUNT_CHANGE_LIMIT = 5
ACCOUNT_CHANGE_WINDOW_SECONDS = 300

_attempt_lock = asyncio.Lock()
_failed_attempts: defaultdict[str, deque] = defaultdict(deque)

_dummy_hash: str | None = None


def _timed_out(key, window_seconds, now):
    values = _failed_attempts[key]
    while values and now - values[0] >= window_seconds:
        values.popleft()
    return values


async def throttle_attempt(key, limit, window_seconds):
    """Raise 429 when the sliding failure window is exhausted for a key."""
    async with _attempt_lock:
        if len(_timed_out(key, window_seconds, _time.monotonic())) >= limit:
            raise HTTPException(429, "試行回数が多すぎます。しばらく待ってから再試行してください。")


async def record_failed_attempt(key, window_seconds):
    async with _attempt_lock:
        _failed_attempts[key].append(_time.monotonic())


def clear_attempts(key):
    _failed_attempts.pop(key, None)


def dummy_password_hash():
    """Precomputed argon2 hash verified against unknown-username logins so the
    password check always runs (removing an account-existence timing signal)."""
    global _dummy_hash
    if _dummy_hash is None:
        _dummy_hash = passwords.hash(secrets.token_hex(16))
    return _dummy_hash


def _get_master_key() -> bytes:
    global _master_key
    if _master_key is None:
        _master_key = (config.KEYS / "master.key").read_bytes()
    return _master_key


def new_session_csrf():
    return secrets.token_urlsafe(32)


def encrypt(value: str, purpose: str):
    nonce = secrets.token_bytes(12)
    cipher = AESGCM(_get_master_key()).encrypt(
        nonce, value.encode(), purpose.encode()
    )
    return {
        "ciphertext": base64.b64encode(cipher).decode(),
        "nonce": base64.b64encode(nonce).decode(),
        "purpose": purpose,
        "key_version": 1,
    }


def decrypt(data):
    return (
        AESGCM(_get_master_key())
        .decrypt(
            base64.b64decode(data["nonce"]), base64.b64decode(data["ciphertext"]), data["purpose"].encode()
        )
        .decode()
    )


def store_secret(db, owner, value, purpose):
    row = Secret(owner_id=owner, data=encrypt(value, purpose))
    db.add(row)
    db.flush()
    return row.id


def read_secret(db, secret_id):
    row = db.get(Secret, secret_id) if secret_id else None
    return decrypt(row.data) if row else ""


def new_session(db, owner, response):
    token = secrets.token_urlsafe(32)
    row = Session(
        id=hashlib.sha256(token.encode()).hexdigest(),
        owner_id=owner,
        csrf=new_session_csrf(),
        expires=now() + timedelta(days=7),
    )
    db.add(row)
    response.set_cookie(
        "mix_session",
        token,
        httponly=True,
        secure=config.COOKIE_SECURE,
        samesite="strict",
        max_age=604800,
        path="/",
    )
    return row


def authenticate(request: Request, db):
    token = request.cookies.get("mix_session", "")
    row = db.get(Session, hashlib.sha256(token.encode()).hexdigest())
    if not row or row.expires.replace(tzinfo=UTC) < now():
        raise HTTPException(401, "ログインしてください")
    if request.method not in ("GET", "HEAD", "OPTIONS") and not secrets.compare_digest(
        request.headers.get("x-csrf-token", ""), row.csrf
    ):
        raise HTTPException(403, "CSRFトークンが一致しません")
    return row
