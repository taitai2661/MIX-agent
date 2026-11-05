"""Idempotency-key lookup must never hand out another owner's/conversation's Run."""

from types import SimpleNamespace

import pytest
from fastapi import HTTPException
from mix_agent.api.message_idempotency import find_existing_run


class FakeDB:
    def __init__(self, run):
        self.run = run

    def scalar(self, _query):
        return self.run


def run(owner="owner", conversation="conv", digest="hash"):
    return SimpleNamespace(
        owner_id=owner,
        conversation_id=conversation,
        data={"request_hash": digest},
    )


def test_returns_the_matching_run():
    existing = run()
    assert find_existing_run(FakeDB(existing), "key", "owner", "conv", "hash") is existing


def test_unknown_key_returns_none():
    assert find_existing_run(FakeDB(None), "key", "owner", "conv", "hash") is None


@pytest.mark.parametrize(
    "existing",
    [
        run(owner="other"),
        run(conversation="other"),
        run(digest="different"),
    ],
)
def test_mismatched_run_is_rejected(existing):
    with pytest.raises(HTTPException) as caught:
        find_existing_run(FakeDB(existing), "key", "owner", "conv", "hash")
    assert caught.value.status_code == 409
