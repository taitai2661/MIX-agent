"""Bounded brute-force protection (H5)."""

from contextlib import contextmanager
from datetime import UTC, datetime, timedelta

import pytest
from fastapi import HTTPException
from mix_agent.auth import security


@pytest.fixture(autouse=True)
def clock(monkeypatch):
    security._failed_attempts.clear()
    clock = {"now": 1000.0}
    monkeypatch.setattr(security._time, "monotonic", lambda: clock["now"])
    # The authoritative counter uses a wall clock, so advancing only the
    # monotonic clock would leave its rows unexpired. Drive both from the same
    # fake clock and keep these unit tests on FakeDB (reset by clean_fake_db)
    # instead of the shared PostgreSQL database.
    epoch = datetime(2026, 1, 1, tzinfo=UTC)
    monkeypatch.setattr(security, "_utcnow", lambda: epoch + timedelta(seconds=clock["now"] - 1000.0))
    monkeypatch.setattr(security, "SessionLocal", FakeDB)
    yield clock
    security._failed_attempts.clear()


async def test_failures_below_limit_do_not_block():
    key = "login:account"
    for _ in range(security.LOGIN_ACCOUNT_LIMIT - 1):
        security.record_failed_attempt(key, security.LOGIN_WINDOW_SECONDS)
    security.throttle_attempt(key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS)


async def test_limit_exhaustion_blocks_then_recovers_after_window(clock):
    key = "login:account"
    for _ in range(security.LOGIN_ACCOUNT_LIMIT):
        security.record_failed_attempt(key, security.LOGIN_WINDOW_SECONDS)
    with pytest.raises(HTTPException):
        security.throttle_attempt(key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS)
    clock["now"] += security.LOGIN_WINDOW_SECONDS + 1
    security.throttle_attempt(key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS)


async def test_window_rolls_off_old_failures(clock):
    key = "change:key"
    for _ in range(security.ACCOUNT_CHANGE_LIMIT):
        security.record_failed_attempt(key, security.ACCOUNT_CHANGE_WINDOW_SECONDS)
    clock["now"] += security.ACCOUNT_CHANGE_WINDOW_SECONDS + 1
    for _ in range(security.ACCOUNT_CHANGE_LIMIT):
        security.record_failed_attempt(key, security.ACCOUNT_CHANGE_WINDOW_SECONDS)
    clock["now"] += security.ACCOUNT_CHANGE_WINDOW_SECONDS + 1
    # Only the first window's entries are dropped; the bucket is now empty.
    security.throttle_attempt(key, security.ACCOUNT_CHANGE_LIMIT, security.ACCOUNT_CHANGE_WINDOW_SECONDS)


async def test_clear_attempts_resets_bucket():
    key = "login:account"
    for _ in range(security.LOGIN_ACCOUNT_LIMIT):
        security.record_failed_attempt(key, security.LOGIN_WINDOW_SECONDS)
    security.clear_attempts(key)
    security.throttle_attempt(key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS)


def test_dummy_hash_is_a_valid_argon2_hash():
    value = security.dummy_password_hash()
    assert value.startswith("$argon2")
    assert security.dummy_password_hash() == value


class FakeRow:
    def __init__(self, id, count, window_start, updated_at):
        self.id = id
        self.count = count
        self.window_start = window_start
        self.updated_at = updated_at

    def __getitem__(self, index):
        return (self.count, self.window_start)[index]


class FakeResult:
    def __init__(self, row=None, rowcount=0):
        self._row = row
        self.rowcount = rowcount

    def first(self):
        return self._row


class FakeDB:
    """Minimal session double that understands the two attempt-counter SQL statements."""

    rows = {}
    statements = []
    lock = None

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        return False

    @contextmanager
    def begin(self):
        if self.lock:
            with self.lock:
                yield
        else:
            yield

    def execute(self, statement, params=None):
        sql = str(statement)
        FakeDB.statements.append(sql)
        head = sql.lstrip().upper()
        if head.startswith("SELECT"):
            return FakeResult(self.rows.get(params["id"]))
        if "ON CONFLICT" in sql:
            counter_id = params["id"]
            row = self.rows.get(counter_id)
            if row is None or row.window_start < params["cutoff"]:
                self.rows[counter_id] = FakeRow(counter_id, 1, params["current"], params["current"])
            else:
                row.count += 1
                row.updated_at = params["current"]
            return FakeResult()
        if head.startswith("DELETE"):
            cutoff = next(
                value for value in statement.compile().params.values() if isinstance(value, datetime)
            )
            removed = 0
            for counter_id, row in list(self.rows.items()):
                if row.window_start < cutoff:
                    del self.rows[counter_id]
                    removed += 1
            return FakeResult(rowcount=removed)
        raise AssertionError(f"unexpected statement: {sql}")

    def get(self, _model, key):
        return self.rows.get(key)

    def delete(self, row):
        self.rows.pop(row.id, None)

    def commit(self):
        pass


@pytest.fixture(autouse=True)
def clean_fake_db():
    FakeDB.rows = {}
    FakeDB.statements = []
    FakeDB.lock = None
    yield
    FakeDB.rows = {}
    FakeDB.statements = []
    FakeDB.lock = None


def test_db_counter_is_authoritative_and_survives_in_memory_reset(monkeypatch):
    """A restart (or second worker) cannot reset a throttled brute-force key."""
    clock = {"wall": datetime(2026, 1, 1, tzinfo=UTC)}
    monkeypatch.setattr(security, "SessionLocal", FakeDB)
    monkeypatch.setattr(security, "_utcnow", lambda: clock["wall"])
    key = "login:db-account"
    for _ in range(security.LOGIN_ACCOUNT_LIMIT):
        security.record_failed_attempt(key, security.LOGIN_WINDOW_SECONDS)
    # Simulate a fresh process: only the DB row remembers the failures.
    security._failed_attempts.clear()
    with pytest.raises(HTTPException):
        security.throttle_attempt(key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS)
    clock["wall"] += timedelta(seconds=security.LOGIN_WINDOW_SECONDS + 1)
    security.throttle_attempt(key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS)
    security.clear_attempts(key)
    assert FakeDB.rows == {}


def test_db_increment_uses_a_single_atomic_upsert():
    sql = str(security.ATTEMPT_UPSERT_SQL)
    assert "INSERT INTO attempt_counters" in sql
    assert "ON CONFLICT (id) DO UPDATE" in sql
    assert "attempt_counters.count + 1" in sql


def test_check_and_record_share_one_transaction(monkeypatch):
    """The lock SELECT and the increment UPSERT happen inside one db.begin()."""
    events = []

    class TrackingDB(FakeDB):
        @contextmanager
        def begin(self):
            events.append("begin")
            yield
            events.append("commit")

        def execute(self, statement, params=None):
            events.append(str(statement).lstrip().upper().split(" ")[0])
            return super().execute(statement, params)

    monkeypatch.setattr(security, "SessionLocal", TrackingDB)
    security.record_failed_attempt("login:txn", security.LOGIN_WINDOW_SECONDS)
    assert events == ["begin", "SELECT", "INSERT", "commit"]


def test_concurrent_failures_do_not_lose_increments(monkeypatch):
    """Twenty parallel failures must produce a count of twenty, not fewer."""
    import threading

    monkeypatch.setattr(security, "SessionLocal", FakeDB)
    FakeDB.lock = threading.Lock()
    key = "login:parallel"
    threads = [
        threading.Thread(target=security.record_failed_attempt, args=(key, security.LOGIN_WINDOW_SECONDS))
        for _ in range(20)
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    row = FakeDB.rows[security._counter_id(key)]
    assert row.count == 20
    upserts = [sql for sql in FakeDB.statements if "ON CONFLICT" in sql]
    assert len(upserts) == 20


def test_purge_expired_attempts_removes_only_stale_rows(monkeypatch):
    monkeypatch.setattr(security, "SessionLocal", FakeDB)
    current = datetime(2026, 1, 1, tzinfo=UTC)
    monkeypatch.setattr(security, "_utcnow", lambda: current)
    FakeDB.rows["stale"] = FakeRow("stale", 9, current - timedelta(hours=2), current)
    FakeDB.rows["fresh"] = FakeRow("fresh", 1, current - timedelta(seconds=5), current)
    with FakeDB() as db:
        assert security.purge_expired_attempts(db, window_seconds=60) == 1
    assert set(FakeDB.rows) == {"fresh"}


def test_encrypt_uses_the_configured_key_version(tmp_path, monkeypatch):
    monkeypatch.setattr(security.config, "KEYS", tmp_path)
    (tmp_path / "master.key").write_bytes(b"v" * 32)
    (tmp_path / "master.key.v2").write_bytes(b"2" * 32)
    monkeypatch.setattr(security, "KEY_VERSION", 2)
    item = security.encrypt("secret", "provider")
    assert item["key_version"] == 2
    assert security.decrypt(item) == "secret"


def test_key_version_comes_from_the_environment(monkeypatch):
    import importlib

    from mix_agent import config

    monkeypatch.setenv("MIX_KEY_VERSION", "3")
    try:
        importlib.reload(config)
        assert config.KEY_VERSION == 3
    finally:
        monkeypatch.delenv("MIX_KEY_VERSION")
        importlib.reload(config)
    assert config.KEY_VERSION == 1


def test_decrypt_selects_key_version(tmp_path, monkeypatch):
    import base64

    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

    monkeypatch.setattr(security, "KEY_RING", {})
    monkeypatch.setattr(security.config, "KEYS", tmp_path)
    (tmp_path / "master.key").write_bytes(b"v" * 32)
    v2 = b"2" * 32
    (tmp_path / "master.key.v2").write_bytes(v2)

    item = security.encrypt("secret", "provider")
    assert item["key_version"] == 1
    assert security.decrypt(item) == "secret"

    nonce = base64.b64encode(b"0123456789ab").decode()
    cipher = AESGCM(v2).encrypt(b"0123456789ab", b"rotated", b"provider")
    old = {
        "ciphertext": base64.b64encode(cipher).decode(),
        "nonce": nonce,
        "purpose": "provider",
        "key_version": 2,
    }
    assert security.decrypt(old) == "rotated"
