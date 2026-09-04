import base64
import hashlib
import secrets
from datetime import timedelta, timezone
from argon2 import PasswordHasher
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from fastapi import HTTPException, Request
from mix_agent import config
from mix_agent.db.models import Session, Secret, now

passwords = PasswordHasher()

_master_key: bytes | None = None


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
    if not row or row.expires.replace(tzinfo=timezone.utc) < now():
        raise HTTPException(401, "ログインしてください")
    if request.method not in ("GET", "HEAD", "OPTIONS"):
        if not secrets.compare_digest(request.headers.get("x-csrf-token", ""), row.csrf):
            raise HTTPException(403, "CSRFトークンが一致しません")
    return row
