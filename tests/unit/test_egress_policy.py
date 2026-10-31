"""The egress policy document is written in exactly one place."""

import json

from mix_agent import config, egress_policy


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
    assert json.loads(path.read_text())["mode"] == "public_web"
