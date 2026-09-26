"""Tests for the new RunCheckpoint module: cadence, payload, and resume."""

import pytest
from mix_agent.db.models import Conversation, Run, RunCheckpoint, User
from mix_agent.db.session import SessionLocal
from mix_agent.runs import checkpoints
from mix_agent.runs.mode_policy import mode_policy
from sqlalchemy import select


def _make_run(mode="agent", steps=0, tool_count=0, plan_steps=None, pending=None):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "Checkpoint"})
        db.add(conversation)
        db.flush()
        policy = dict(mode_policy(mode))
        snapshot = {"mode": mode, "policy": policy, "max_seconds": 5400, "max_steps": 300, "max_tool_calls": 750}
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key="ckpt-key-1",
            data={
                "snapshot": snapshot,
                "history": [{"role": "system", "content": "sys"}, {"role": "user", "content": "do it"}],
                "task_state": {"goal": "ship", "pending": pending or [], "plan": plan_steps or ["phase:setup"]},
                "agent_plan": {"steps": plan_steps or ["phase:setup"], "pending": pending or [], "verification": ""},
                "steps": steps,
                "tool_count": tool_count,
                "summary": {"text": "old summary", "covered_count": 4, "updated_at": None},
                "context_trace": {"estimated_input_tokens": 123},
                "missing_image_refs": [],
                "tool_refs": [],
            },
        )
        db.add(run)
        db.commit()
        return run.id


def test_should_checkpoint_respects_cadence():
    snapshot = {"policy": {"checkpoint_every_steps": 5}}
    assert checkpoints.should_checkpoint(snapshot, 1) is True
    assert checkpoints.should_checkpoint(snapshot, 4) is False
    assert checkpoints.should_checkpoint(snapshot, 5) is True
    assert checkpoints.should_checkpoint(snapshot, 6) is False
    assert checkpoints.should_checkpoint(snapshot, 0) is False


def test_save_persists_payload_and_trims_old_rows(signed):
    run_id = _make_run(steps=12, tool_count=4)
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        # Insert 25 checkpoints directly to exercise the trim path.
        for index in range(25):
            ck = RunCheckpoint(
                owner_id=run.owner_id,
                run_id=run.id,
                step=index,
                tool_count=index,
                trigger="interval",
                data={"step": index, "tool_count": index},
            )
            db.add(ck)
        db.commit()
        run = db.get(Run, run_id)
        latest = checkpoints.save(db, run, trigger="interval")
        db.commit()
        kept = checkpoints.list_for_run(db, run.id)
    # Default keep_recent = 20; the 25 pre-existing + 1 new = 26, trimmed to 20.
    assert len(kept) == 20
    assert latest.id in {row["id"] for row in kept}
    # Latest checkpoint should appear first in the listing.
    assert kept[0]["id"] == latest.id
    assert kept[0]["trigger"] == "interval"


def test_apply_to_run_restores_history_and_counters(signed):
    run_id = _make_run(steps=42, tool_count=10, pending=["wrap-up"])
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        snapshot = checkpoints.capture_snapshot(db, run, trigger="manual")
        ck = RunCheckpoint(
            owner_id=run.owner_id,
            run_id=run.id,
            step=snapshot["step"],
            tool_count=snapshot["tool_count"],
            trigger="manual",
            data=snapshot,
        )
        db.add(ck)
        db.commit()
        ck_id = ck.id

        # Now mutate the run state to simulate progress after checkpoint.
        run.data = {**run.data, "steps": 99, "tool_count": 50, "history": [{"role": "user", "content": "newer"}]}
        db.commit()

        restored = db.get(RunCheckpoint, ck_id)
        run = db.get(Run, run_id)
        checkpoints.apply_to_run(run, restored)
        db.commit()

        run = db.get(Run, run_id)
    assert run.data["steps"] == 42
    assert run.data["tool_count"] == 10
    # Head of history is preserved so the system block survives.
    assert run.data["history"][0]["role"] == "system"
    assert run.data["checkpoint_resumed_from"]["id"] == ck_id


def test_apply_to_run_rejects_wrong_run(signed):
    run_id = _make_run()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        ck = RunCheckpoint(
            owner_id=run.owner_id,
            run_id=run.id,
            step=1, tool_count=0, trigger="manual",
            data={"step": 1, "tool_count": 0},
        )
        db.add(ck)
        db.commit()
        # Foreign run id should not resolve a checkpoint.
        assert checkpoints.get(db, "non-existent-run", ck.id) is None
