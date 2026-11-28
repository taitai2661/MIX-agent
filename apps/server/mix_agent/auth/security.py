import base64
import hashlib
import logging
import secrets
import threading
import time as _time
from collections import defaultdict, deque
from datetime import UTC, datetime, timedelta

from argon2 import PasswordHasher
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from fastapi import HTTPException, Request
from sqlalchemy import delete, text

from mix_agent import config
from mix_agent.db.models import AttemptCounter, Secret, Session, now
from mix_agent.db.session import SessionLocal

passwords = PasswordHasher()
LOGGER = logging.getLogger(__name__)

# Versioned keyring: ciphertext carries `key_version`, and decrypt selects the
# matching key. Version 1 is the current master.key; later versions live in
# master.key.v<N> so old secrets stay readable after a rotation. The *current*
# write version comes from MIX_KEY_VERSION (config.KEY_VERSION), so rotating
# does not require a code change.
KEY_RING: dict[int, bytes] = {}
KEY_VERSION = config.KEY_VERSION

# One DB transaction checks the counter and (for failures) increments it. The
# row is locked for the read and the increment is a single INSERT ... ON
# CONFLICT DO UPDATE SET count = count + 1 statement: a read-modify-write in
# Python lost increments under concurrent failures on different workers.
ATTEMPT_LOCK_SQL = text("SELECT count, window_start FROM attempt_counters WHERE id = :id FOR UPDATE")
ATTEMPT_UPSERT_SQL = text("""
    INSERT INTO attempt_counters (id, count, window_start, updated_at)
    VALUES (:id, 1, :current, :current)
    ON CONFLICT (id) DO UPDATE
    SET count = CASE WHEN attempt_counters.window_start < :cutoff THEN 1
                     ELSE attempt_counters.count + 1 END,
        window_start = CASE WHEN attempt_counters.window_start < :cutoff THEN :current
                            ELSE attempt_counters.window_start END,
        updated_at = :current
""")
# Recording is not itself an auth decision; the caller has already classified
# the attempt as failed. A high limit keeps the shared check from rejecting
# here while still performing the atomic increment in the same transaction.
_RECORD_ONLY_LIMIT = 1 << 30

# Uppercase: bounded brute-force protection for auth surfaces.  Failures only;
# successful operations never consume or write to these in-memory buckets.
LOGIN_WINDOW_SECONDS = 300
LOGIN_ACCOUNT_LIMIT = 10
LOGIN_IP_LIMIT = 20
ACCOUNT_CHANGE_LIMIT = 5
ACCOUNT_CHANGE_WINDOW_SECONDS = 300

_attempt_lock = threading.Lock()
_failed_attempts: defaultdict[str, deque] = defaultdict(deque)

_dummy_hash: str | None = None


def _timed_out(key, window_seconds, now):
    values = _failed_attempts[key]
    while values and now - values[0] >= window_seconds:
        values.popleft()
    return values


def _prune_empty(now):
    """Drop buckets that have fully aged out.

    Keys are derived from account ids and client IPs, so without this the
    process keeps one empty deque per address for its lifetime.
    """
    longest = max(LOGIN_WINDOW_SECONDS, ACCOUNT_CHANGE_WINDOW_SECONDS)
    for key in list(_failed_attempts):
        if not _timed_out(key, longest, now):
            _failed_attempts.pop(key, None)


def _utcnow():
    """Injectable wall clock for the DB-backed counter (tests advance it)."""
    return datetime.now(UTC)


def _counter_id(key):
    return hashlib.sha256(key.encode()).hexdigest()


def _db_attempt(key, limit, window_seconds, *, record):
    """Check the limit and optionally record a failure in ONE transaction.

    ``SELECT ... FOR UPDATE`` makes the check see a stable row while the
    increment is a single atomic upsert, so two workers cannot both read the
    same count and lose an increment. Returns True when the key is throttled.
    A DB failure degrades to the in-memory bucket (telemetry must never fail an
    auth request).
    """
    current = _utcnow()
    cutoff = current - timedelta(seconds=window_seconds)
    counter_id = _counter_id(key)
    try:
        with SessionLocal() as db:
            with db.begin():
                locked = db.execute(ATTEMPT_LOCK_SQL, {"id": counter_id}).first()
                count = 0
                if locked is not None and locked[1] >= cutoff:
                    count = int(locked[0])
                if count >= limit:
                    return True
                if record:
                    db.execute(
                        ATTEMPT_UPSERT_SQL,
                        {"id": counter_id, "current": current, "cutoff": cutoff},
                    )
        return False
    except Exception:  # noqa: BLE001 - in-memory bucket is the degraded fallback
        LOGGER.debug("DB attempt counter check failed", exc_info=True)
        return False


def _db_clear(key):
    try:
        with SessionLocal() as db:
            row = db.get(AttemptCounter, _counter_id(key))
            if row is not None:
                db.delete(row)
                db.commit()
    except Exception:  # noqa: BLE001 - best-effort reset
        LOGGER.debug("DB attempt counter reset failed", exc_info=True)


def throttle_attempt(key, limit, window_seconds):
    """Raise 429 when the failure window is exhausted for a key.

    The in-memory bucket is a cheap pre-check; PostgreSQL is authoritative so a
    restart or a second worker cannot reset the limit. Both paths stay
    synchronous so auth endpoints can keep running in FastAPI's threadpool.
    """
    now = _time.monotonic()
    with _attempt_lock:
        _prune_empty(now)
        if len(_timed_out(key, window_seconds, now)) >= limit:
            raise HTTPException(429, "試行回数が多すぎます。しばらく待ってから再試行してください。")
    if _db_attempt(key, limit, window_seconds, record=False):
        raise HTTPException(429, "試行回数が多すぎます。しばらく待ってから再試行してください。")


def record_failed_attempt(key, window_seconds):
    now = _time.monotonic()
    with _attempt_lock:
        _prune_empty(now)
        _failed_attempts[key].append(now)
    # Check + increment share a single DB transaction inside _db_attempt.
    _db_attempt(key, _RECORD_ONLY_LIMIT, window_seconds, record=True)


def clear_attempts(key):
    with _attempt_lock:
        _failed_attempts.pop(key, None)
    _db_clear(key)


def purge_expired_sessions(db) -> int:
    """Delete expired login sessions; called from the scheduler tick."""
    result = db.execute(delete(Session).where(Session.expires < now()))
    return result.rowcount or 0


def purge_expired_attempts(db, window_seconds=None) -> int:
    """Delete counters whose window has ended; called from the scheduler tick.

    Without this, keys derived from client IPs (`login-ip:<ip>`) would create a
    permanent row per address and grow the table without bound.
    """
    seconds = window_seconds or max(LOGIN_WINDOW_SECONDS, ACCOUNT_CHANGE_WINDOW_SECONDS)
    cutoff = _utcnow() - timedelta(seconds=seconds)
    result = db.execute(delete(AttemptCounter).where(AttemptCounter.window_start < cutoff))
    return result.rowcount or 0


def verify_password(user, password: str) -> bool:
    """Verify without leaking hash/format details to the caller."""
    try:
        return passwords.verify(user.password_hash, password)
    except Exception:  # noqa: BLE001 - intentionally classified
        return False


def dummy_password_hash():
    """Precomputed argon2 hash verified against unknown-username logins so the
    password check always runs (removing an account-existence timing signal)."""
    global _dummy_hash
    if _dummy_hash is None:
        _dummy_hash = passwords.hash(secrets.token_hex(16))
    return _dummy_hash


def _get_master_key(version: int | None = None) -> bytes:
    """Return the key for ``version`` (defaults to the current KEY_VERSION).

    Resolving the default at call time — not as a function default — means
    setting MIX_KEY_VERSION (or overriding it in tests) actually rotates the
    key used by ``encrypt()``.
    """
    if version is None:
        version = KEY_VERSION
    if version not in KEY_RING:
        path = config.KEYS / "master.key" if version == 1 else config.KEYS / f"master.key.v{version}"
        KEY_RING[version] = path.read_bytes()
    return KEY_RING[version]


def new_session_csrf():
    return secrets.token_urlsafe(32)


def encrypt(value: str, purpose: str):
    nonce = secrets.token_bytes(12)
    cipher = AESGCM(_get_master_key()).encrypt(nonce, value.encode(), purpose.encode())
    return {
        "ciphertext": base64.b64encode(cipher).decode(),
        "nonce": base64.b64encode(nonce).decode(),
        "purpose": purpose,
        "key_version": KEY_VERSION,
    }


def decrypt(data):
    version = int(data.get("key_version", 1))
    return (
        AESGCM(_get_master_key(version))
        .decrypt(
            base64.b64decode(data["nonce"]), base64.b64decode(data["ciphertext"]), data["purpose"].encode()
        )
        .decode()
    )


def store_secret(db, owner, value, purpose, previous_id=None):
    """Store an encrypted value, replacing ``previous_id`` in place when possible.

    Rotation used to insert a new row and leave the old ciphertext behind, so
    API-key changes and OAuth refreshes grew the secrets table forever and kept
    decryptable credentials around after they were replaced.
    """
    if previous_id:
        row = db.get(Secret, previous_id)
        if row is not None and row.owner_id == owner:
            row.data = encrypt(value, purpose)
            db.flush()
            return row.id
    row = Secret(owner_id=owner, data=encrypt(value, purpose))
    db.add(row)
    db.flush()
    return row.id


def store_optional_secret(db, owner, value, purpose, previous_id=None):
    """Store a secret, or delete the previous one when the value is cleared."""
    if value:
        return store_secret(db, owner, value, purpose, previous_id)
    if previous_id:
        row = db.get(Secret, previous_id)
        if row is not None and row.owner_id == owner:
            db.delete(row)
    return None


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
    if request.method not in ("GET", "HEAD", "OPTIONS"):
        provided = request.headers.get("x-csrf-token", "")
        # compare_digest raises TypeError on non-ASCII str, which surfaced as a
        # 500 for a malformed header. Reject it as the 403 it is.
        if not provided.isascii() or not secrets.compare_digest(provided, row.csrf):
            raise HTTPException(403, "CSRFトークンが一致しません")
    return row
