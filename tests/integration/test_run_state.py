"""Database-backed tests for the Run state machine and DB source-of-truth rules."""

import pytest
from mix_agent.db.models import Approval, Conversation, Event, Run, User, uid
from mix_agent.db.session import SessionLocal
from mix_agent.runs import engine
from mix_agent.runs.state import InvalidRunTransition, transition_run
from mix_agent.tools.registry import BUILTINS
from sqlalchemy import select
from sqlalchemy.exc import IntegrityError


def make_run(tool_name=None, mode="agent"):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "State Machine Test"})
        db.add(conversation)
        db.flush()
        tools = []
        if tool_name:
            tool = next(t for t in BUILTINS if t["id"] == tool_name)
            tools = [tool]
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key=uid(),
            data={
                "snapshot": {
                    "mode": mode,
                    "model_id": "fake",
                    "provider": {"kind": "compatible"},
                    "tool_ids": [tool_name] if tool_name else [],
                    "tools": tools,
                },
                "history": [{"role": "user", "content": "State machine test"}],
            },
        )
        db.add(run)
        db.commit()
        return run.id


def set_status(run_id, status):
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        run.status = status
        db.commit()


def status_events(run_id):
    with SessionLocal() as db:
        return list(
            db.scalars(
                select(Event)
                .where(Event.run_id == run_id, Event.kind == "status")
                .order_by(Event.sequence)
            )
        )


@pytest.mark.parametrize(
    ("from_status", "to_status"),
    [
        ("queued", "running"),
        ("running", "waiting_approval"),
        ("waiting_approval", "running"),
        ("running", "completed"),
        ("running", "failed"),
        ("running", "cancelled"),
        ("running", "interrupted"),
    ],
)
def test_valid_transitions_update_db_and_emit_status_event(signed, from_status, to_status):
    run_id = make_run()
    set_status(run_id, from_status)
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert transition_run(db, run, to_status, reason="reason-text") is True
        db.commit()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == to_status
        assert run.data.get("reason") == "reason-text"
    events = status_events(run_id)
    assert len(events) == 1
    assert events[0].data["status"] == to_status
    assert events[0].data["from_status"] == from_status
    assert events[0].data["reason"] == "reason-text"


@pytest.mark.parametrize(
    ("from_status", "to_status"),
    [
        ("completed", "running"),
        ("failed", "running"),
        ("cancelled", "running"),
        ("interrupted", "running"),
        ("queued", "completed"),
    ],
)
def test_invalid_transitions_are_rejected_without_db_change_or_event(signed, from_status, to_status):
    run_id = make_run()
    set_status(run_id, from_status)
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        with pytest.raises(InvalidRunTransition):
            transition_run(db, run, to_status, reason="should not persist")
        db.rollback()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == from_status
        assert run.data.get("reason") is None
    assert status_events(run_id) == []


def test_invalid_status_value_is_rejected_by_db_constraint(signed):
    run_id = make_run()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        run.status = "paused"
        with pytest.raises(IntegrityError):
            db.commit()
        db.rollback()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == "queued"
        with pytest.raises(InvalidRunTransition):
            transition_run(db, run, "paused")


def test_same_status_retransition_is_an_explicit_noop(signed):
    run_id = make_run()
    set_status(run_id, "running")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert transition_run(db, run, "running") is False
        db.commit()
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "running"
    assert status_events(run_id) == []


class CompletingAdapter:
    def __init__(self, *_):
        pass

    async def stream(self, model, history, tools, mode, settings):
        yield {"kind": "text", "text": "done"}
        yield {"kind": "response", "message": {"role": "assistant", "content": "done"}, "tool_calls": []}


async def test_drive_terminal_run_does_not_return_to_running(signed, monkeypatch):
    monkeypatch.setattr(engine, "Adapter", CompletingAdapter)
    run_id = make_run()
    set_status(run_id, "completed")
    await engine.drive(run_id)
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "completed"
    events = status_events(run_id)
    assert not any(
        e.data.get("from_status") == "queued" and e.data.get("status") == "running" for e in events
    )


async def test_drive_queued_run_transitions_to_running(signed, monkeypatch):
    monkeypatch.setattr(engine, "Adapter", CompletingAdapter)
    run_id = make_run()
    await engine.drive(run_id)
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "completed"
    events = status_events(run_id)
    assert any(
        e.data.get("from_status") == "queued" and e.data.get("status") == "running" for e in events
    )


async def test_drive_waiting_approval_run_returns_to_running(signed, monkeypatch):
    monkeypatch.setattr(engine, "Adapter", CompletingAdapter)
    run_id = make_run()
    set_status(run_id, "waiting_approval")
    await engine.drive(run_id)
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "completed"
    events = status_events(run_id)
    assert any(
        e.data.get("from_status") == "waiting_approval" and e.data.get("status") == "running"
        for e in events
    )


async def test_drive_does_not_run_from_unexpected_status(signed, monkeypatch):
    """A status drive() was never meant to open must not be overwritten."""
    calls = []

    async def forbidden(*args):
        calls.append(args)

    monkeypatch.setattr(engine, "Adapter", forbidden)
    run_id = make_run()
    set_status(run_id, "running")
    await engine.drive(run_id)
    assert calls == []


async def test_scheduler_startup_interruption_goes_through_state_machine(signed):
    run_id = make_run()
    set_status(run_id, "running")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        engine.finish(db, run, "interrupted", "サーバーが再起動しました。結果不明の操作は自動再実行しません。")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == "interrupted"
    events = status_events(run_id)
    assert events[-1].data["status"] == "interrupted"
    assert events[-1].data["from_status"] == "running"
    await engine.drive(run_id)
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "interrupted"


async def test_approval_flow_uses_state_machine_and_executes_once(signed, monkeypatch):
    run_id = make_run(tool_name="write_file", mode="agent")
    executions = []
    stream_calls = {"n": 0}

    class ToolRequestingAdapter:
        def __init__(self, *_):
            pass

        async def stream(self, model, history, tools, mode, settings):
            stream_calls["n"] += 1
            if stream_calls["n"] == 1:
                name = tools[0]["model_name"]
                yield {"kind": "text", "text": "progress"}
                yield {
                    "kind": "response",
                    "message": {"role": "assistant", "content": "I will write a file"},
                    "tool_calls": [
                        {"id": "c1", "name": name, "arguments": {"path": "hello.txt", "content": "hello"}}
                    ],
                }
            else:
                yield {"kind": "text", "text": "done"}
                yield {"kind": "response", "message": {"role": "assistant", "content": "The file was written."}}

    async def execute(*args):
        executions.append(args[-1])
        return {"ok": True}

    monkeypatch.setattr(engine, "Adapter", ToolRequestingAdapter)
    monkeypatch.setattr(engine, "execute", execute)
    await engine.drive(run_id)
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == "waiting_approval"
        approval = db.scalar(select(Approval).where(Approval.run_id == run_id))
        assert approval is not None
        approval.status = "once"
        db.commit()
    assert executions == []
    await engine.drive(run_id)
    assert len(executions) == 1
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == "interrupted"
        assert "作業計画" in run.data["reason"]
    events = status_events(run_id)
    assert any(
        e.data.get("from_status") == "running" and e.data.get("status") == "waiting_approval"
        for e in events
    )
    assert any(
        e.data.get("from_status") == "waiting_approval" and e.data.get("status") == "running"
        for e in events
    )


def test_one_active_run_index_still_holds(signed):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "Active"})
        db.add(conversation)
        db.flush()
        for _ in range(2):
            db.add(
                Run(
                    owner_id=owner,
                    conversation_id=conversation.id,
                    request_key=uid(),
                    data={"snapshot": {"mode": "chat"}, "history": []},
                )
            )
        with pytest.raises(IntegrityError):
            db.commit()
        db.rollback()


def test_terminal_states_are_never_overwritten_by_finish(signed):
    run_id = make_run()
    set_status(run_id, "completed")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        engine.finish(db, run, "failed", "should not apply")
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "completed"
    assert status_events(run_id) == []


def test_transition_is_committed_atomically_with_event(signed):
    """The status update and its Event share one commit; no state/event skew."""
    run_id = make_run()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        transition_run(db, run, "running")
        db.commit()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        transition_run(db, run, "completed")
        db.commit()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == "completed"
        statuses = db.scalars(
            select(Event)
            .where(Event.run_id == run_id, Event.kind == "status")
            .order_by(Event.sequence)
        ).all()
        assert [e.data["status"] for e in statuses] == ["running", "completed"]
        assert statuses[0].data["from_status"] == "queued"
