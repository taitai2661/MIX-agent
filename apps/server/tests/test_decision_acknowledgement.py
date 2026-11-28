# ruff: noqa: S101 - this is a test module (root ruff.toml's tests/** ignore does
# not reach apps/server/tests/**).
"""Server-side acknowledgement gate for risky approval decisions.

Kept under ``apps/server`` per the file-ownership restriction for this change;
run explicitly with ``pytest -q apps/server/tests/test_decision_acknowledgement.py``.
"""

from datetime import UTC, datetime, timedelta
from types import SimpleNamespace

import pytest
from fastapi import HTTPException
from mix_agent.api import routes_runs
from mix_agent.api.schemas import DecisionInput
from mix_agent.db.models import Approval, Run


class _Approval:
    id = "ap-1"
    status = "pending"
    run_id = "run-1"
    tool_call_id = "tc-1"
    expires = datetime.now(UTC) + timedelta(hours=1)

    def __init__(self, risk):
        self.data = {"risk": risk, "tool_version": "v1", "scope": {"tool_id": "terminal"}}


class _Run:
    id = "run-1"
    status = "waiting_approval"
    data = {"snapshot": {}}


class _DB:
    def __init__(self):
        self.added = []
        self.committed = False

    def get(self, _model, _key):
        return SimpleNamespace(data={"tool_id": "terminal"})

    def add(self, row):
        self.added.append(row)

    def commit(self):
        self.committed = True


@pytest.fixture
def context(monkeypatch):
    """A minimal approval context; no PostgreSQL required."""
    approval = {"risk": "write"}

    def fake_own(_db, model, _key, _owner):
        if model is Approval:
            return _Approval(approval["risk"])
        if model is Run:
            return _Run()
        raise AssertionError(f"unexpected model {model}")

    class Registry:
        def get(self, _tool_id):
            return {"id": "terminal"}

    monkeypatch.setattr(routes_runs, "own", fake_own)
    monkeypatch.setattr(routes_runs, "registry", lambda _db, _owner: Registry())
    monkeypatch.setattr(routes_runs, "fingerprint", lambda _tool: "v1")
    monkeypatch.setattr(routes_runs, "audit", lambda *_args, **_kwargs: None)
    monkeypatch.setattr(routes_runs, "launch", lambda *_args, **_kwargs: None)
    return approval


def _decide(decision, **extra):
    db = _DB()
    result = routes_runs.decide(
        "ap-1",
        DecisionInput.model_validate({"decision": decision, **extra}),
        (db, SimpleNamespace(owner_id="user-1")),
    )
    return db, result


def test_schema_accepts_both_acknowledgement_spellings():
    assert (
        DecisionInput.model_validate({"decision": "once", "acknowledgeRisk": True}).acknowledge_risk is True
    )
    assert (
        DecisionInput.model_validate({"decision": "once", "acknowledge_risk": True}).acknowledge_risk is True
    )
    assert DecisionInput.model_validate({"decision": "once"}).acknowledge_risk is False


@pytest.mark.parametrize("decision", ["once", "always"])
@pytest.mark.parametrize("risk", ["write", "external", "dangerous"])
def test_risky_allow_without_acknowledgement_is_rejected(context, decision, risk):
    context["risk"] = risk
    with pytest.raises(HTTPException) as excinfo:
        _decide(decision)
    assert excinfo.value.status_code == 422
    assert "リスクの確認" in excinfo.value.detail


def test_risky_one_off_allow_with_acknowledgement_succeeds(context):
    context["risk"] = "dangerous"
    db, result = _decide("once", acknowledgeRisk=True)
    assert result == {"ok": True}
    assert db.committed
    assert db.added == []


@pytest.mark.parametrize("risk", ["write", "external", "dangerous"])
def test_risky_persistent_allow_is_rejected_even_with_acknowledgement(context, risk):
    """The UI hides 'always allow' for elevated risks; the API must enforce it."""
    context["risk"] = risk
    with pytest.raises(HTTPException) as excinfo:
        _decide("always", acknowledgeRisk=True)
    assert excinfo.value.status_code == 422
    assert "常時許可" in excinfo.value.detail


@pytest.mark.parametrize("risk", ["read", None])
def test_non_risky_allow_without_acknowledgement_succeeds(context, risk):
    context["risk"] = risk
    db, result = _decide("once")
    assert result == {"ok": True}
    assert db.committed


def test_non_risky_persistent_allow_is_still_allowed(context):
    context["risk"] = "read"
    db, result = _decide("always")
    assert result == {"ok": True}
    assert db.committed
    assert len(db.added) == 1  # a Permission row is created


@pytest.mark.parametrize("risk", ["write", "external", "dangerous"])
def test_deny_is_unaffected(context, risk):
    context["risk"] = risk
    db, result = _decide("denied")
    assert result == {"ok": True}
    assert db.committed
    assert db.added == []
