"""Bounded brute-force protection (H5)."""

import pytest
from fastapi import HTTPException
from mix_agent.auth import security


@pytest.fixture(autouse=True)
def clock(monkeypatch):
    security._failed_attempts.clear()
    clock = {"now": 1000.0}
    monkeypatch.setattr(security._time, "monotonic", lambda: clock["now"])
    yield clock
    security._failed_attempts.clear()


async def test_failures_below_limit_do_not_block():
    key = "login:account"
    for _ in range(security.LOGIN_ACCOUNT_LIMIT - 1):
        await security.record_failed_attempt(key, security.LOGIN_WINDOW_SECONDS)
    await security.throttle_attempt(
        key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS
    )


async def test_limit_exhaustion_blocks_then_recovers_after_window(clock):
    key = "login:account"
    for _ in range(security.LOGIN_ACCOUNT_LIMIT):
        await security.record_failed_attempt(key, security.LOGIN_WINDOW_SECONDS)
    with pytest.raises(HTTPException):
        await security.throttle_attempt(
            key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS
        )
    clock["now"] += security.LOGIN_WINDOW_SECONDS + 1
    await security.throttle_attempt(
        key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS
    )


async def test_window_rolls_off_old_failures(clock):
    key = "change:key"
    for _ in range(security.ACCOUNT_CHANGE_LIMIT):
        await security.record_failed_attempt(key, security.ACCOUNT_CHANGE_WINDOW_SECONDS)
    clock["now"] += security.ACCOUNT_CHANGE_WINDOW_SECONDS + 1
    for _ in range(security.ACCOUNT_CHANGE_LIMIT):
        await security.record_failed_attempt(key, security.ACCOUNT_CHANGE_WINDOW_SECONDS)
    clock["now"] += security.ACCOUNT_CHANGE_WINDOW_SECONDS + 1
    # Only the first window's entries are dropped; the bucket is now empty.
    await security.throttle_attempt(
        key, security.ACCOUNT_CHANGE_LIMIT, security.ACCOUNT_CHANGE_WINDOW_SECONDS
    )


async def test_clear_attempts_resets_bucket():
    key = "login:account"
    for _ in range(security.LOGIN_ACCOUNT_LIMIT):
        await security.record_failed_attempt(key, security.LOGIN_WINDOW_SECONDS)
    security.clear_attempts(key)
    await security.throttle_attempt(
        key, security.LOGIN_ACCOUNT_LIMIT, security.LOGIN_WINDOW_SECONDS
    )


def test_dummy_hash_is_a_valid_argon2_hash():
    value = security.dummy_password_hash()
    assert value.startswith("$argon2")
    assert security.dummy_password_hash() == value