"""The egress policy document is written in exactly one place."""

import json

import pytest

from mix_agent import config, egress_policy
from mix_agent.api.error_codes import APIError


def test_empty_allow_list_is_an_explicit_public_web_policy(tmp_path, monkeypatch):
    monkeypatch.setattr(config, "DATA", tmp_path)
    path = egress_policy.write_policy([])
    assert json.loads(path.read_text()) == {"mode": "public_web", "allowed_domains": []}


def test_allowed_domains_switch_to_restricted_mode(tmp_path, monkeypatch):
    monkeypatch.setattr(config, "DATA", tmp_path)
    path = egress_policy.write_policy(["Example.com"])
    assert json.loads(path.read_text()) == {
        "mode": "restricted",
        "allowed_domains": ["Example.com"],
    }


def test_policy_write_is_atomic_and_idempotent(tmp_path, monkeypatch):
    monkeypatch.setattr(config, "DATA", tmp_path)
    egress_policy.write_policy([])
    path = egress_policy.write_policy([])
    assert not (tmp_path / "egress" / "policy.tmp").exists()
    assert list((tmp_path / "egress").glob("policy.*.tmp")) == []
    assert json.loads(path.read_text())["mode"] == "public_web"


def test_a_write_failure_becomes_a_stable_api_error(monkeypatch):
    def fail(_domains):
        raise OSError("disk full")

    monkeypatch.setattr(egress_policy, "write_policy", fail)
    with pytest.raises(APIError) as excinfo:
        egress_policy.write_policy_or_503(["example.com"])
    assert excinfo.value.status_code == 503
    assert excinfo.value.code == "egress.policy_write_failed"


def test_a_post_commit_write_failure_does_not_raise(monkeypatch):
    """A restore has already committed when this runs; never undo it."""

    def fail(_domains):
        raise OSError("disk full")

    monkeypatch.setattr(egress_policy, "write_policy", fail)
    assert egress_policy.write_policy_best_effort(["example.com"]) is False


def test_startup_reconcile_rewrites_the_policy_from_settings(tmp_path, monkeypatch):
    from types import SimpleNamespace

    from mix_agent.db import session as db_session

    class FakeSession:
        def __enter__(self):
            return self

        def __exit__(self, *_exc):
            return False

        def get(self, model, key):
            assert key == "settings"
            return SimpleNamespace(data={"allowed_domains": ["example.com"]})

    monkeypatch.setattr(db_session, "SessionLocal", FakeSession)
    monkeypatch.setattr(egress_policy.config, "DATA", tmp_path)
    assert egress_policy.reconcile_from_db() is True
    policy = json.loads((tmp_path / "egress" / "policy.json").read_text(encoding="utf-8"))
    assert policy == {"mode": "restricted", "allowed_domains": ["example.com"]}
