"""Tests for the budget_extension_pending status transitions and policies."""

import pytest
from mix_agent.db.models import Conversation, Run, User
from mix_agent.db.session import SessionLocal
from mix_agent.runs.mode_policy import apply_mode_defaults, mode_policy
from mix_agent.runs.state import InvalidRunTransition, is_valid_status, transition_run
from sqlalchemy import select


def _make_run(mode="agent", status="running"):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "Budget"})
        db.add(conversation)
        db.flush()
        snapshot = apply_mode_defaults({"mode": mode, "tools": [], "tool_ids": []}, mode)
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key="budget-key",
            status=status,
            data={
                "snapshot": snapshot,
                "history": [{"role": "user", "content": "build it"}],
            },
        )
        db.add(run)
        db.commit()
        return run.id


def test_budget_extension_is_a_known_status():
    assert is_valid_status("budget_extension_pending")


def test_running_to_budget_extension_is_valid(signed):
    run_id = _make_run(status="running")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert transition_run(db, run, "budget_extension_pending") is True
        db.commit()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == "budget_extension_pending"


def test_budget_extension_to_running_is_valid(signed):
    run_id = _make_run(status="budget_extension_pending")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert transition_run(db, run, "running", expected_from="budget_extension_pending") is True
        db.commit()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == "running"


def test_terminal_to_budget_extension_is_rejected(signed):
    run_id = _make_run(status="completed")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        with pytest.raises(InvalidRunTransition):
            transition_run(db, run, "budget_extension_pending")
        db.rollback()


def test_agent_mode_policy_carries_new_flags():
    policy = mode_policy("agent")
    assert policy["budget_extension"] is True
    assert policy["stagnation_detection"] is True
    assert policy["strict_verification"] is True
    assert policy["checkpoint_every_steps"] >= 1
    assert policy["max_budget_extensions"] >= 1


def test_chat_mode_policy_does_not_enable_agent_features():
    policy = mode_policy("chat")
    assert policy["budget_extension"] is False
    assert policy["stagnation_detection"] is False
    assert policy["strict_verification"] is False
    assert policy["checkpoint_every_steps"] == 0
