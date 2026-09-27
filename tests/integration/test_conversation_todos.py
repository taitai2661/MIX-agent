"""Conversation-scoped todos: mirroring, API exposure, and prompt carry-over."""

from sqlalchemy import select

from mix_agent.api import routes
from mix_agent.db.models import Conversation, Model, Provider, Run, ToolCall, User, uid
from mix_agent.db.session import SessionLocal
from mix_agent.runs import engine
from mix_agent.tools.registry import BUILTINS


def make_agent_run():
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "Todo"})
        db.add(conversation)
        db.flush()
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key=uid(),
            data={
                "snapshot": {
                    "mode": "agent",
                    "model_id": "fake",
                    "provider": {"kind": "compatible"},
                    "tool_ids": ["update_plan"],
                    "tools": [next(tool for tool in BUILTINS if tool["id"] == "update_plan")],
                },
                "history": [{"role": "user", "content": "Do the work"}],
            },
        )
        db.add(run)
        db.commit()
        return run.id


def complete_plan(run_id, result, index=0):
    tools = {tool["id"]: tool for tool in BUILTINS}
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        call = ToolCall(
            owner_id=run.owner_id,
            run_id=run.id,
            data={
                "tool_id": "update_plan",
                "name": "update_plan",
                "risk": tools["update_plan"]["risk"],
                "provider_call_id": f"todo-{index}",
                "arguments": {},
            },
        )
        db.add(call)
        db.flush()
        engine.complete_tool_call(db, run, call, tools["update_plan"], result)
        db.commit()
        return run.conversation_id


def create_model():
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "openai"})
        db.add(provider)
        db.flush()
        model = Model(
            owner_id=owner,
            data={
                "provider_id": provider.id,
                "model_id": "test-todos",
                "capabilities": {"reasoning": True, "tools": True},
            },
        )
        db.add(model)
        db.commit()
        return model.id


def test_update_plan_mirrors_todos_onto_conversation(signed):
    conversation_id = complete_plan(
        make_agent_run(),
        {"steps": ["phase:setup", "編集", "検証"], "pending": ["編集", "検証"], "verification": ""},
    )
    with SessionLocal() as db:
        conversation = db.get(Conversation, conversation_id)
        assert conversation.data["todos"] == [
            {"content": "phase:setup", "status": "completed"},
            {"content": "編集", "status": "in_progress"},
            {"content": "検証", "status": "pending"},
        ]


def test_update_plan_with_empty_steps_clears_todos(signed):
    run_id = make_agent_run()
    conversation_id = complete_plan(run_id, {"steps": ["残"], "pending": ["残"]}, index=0)
    complete_plan(run_id, {"steps": [], "pending": []}, index=1)
    with SessionLocal() as db:
        conversation = db.get(Conversation, conversation_id)
        assert conversation.data["todos"] == []


def test_messages_endpoint_returns_sanitized_todos(signed):
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    with SessionLocal() as db:
        row = db.get(Conversation, conversation)
        row.data = {
            **row.data,
            "todos": [
                {"content": "残作業", "status": "pending"},
                {"content": "壊れた項目", "status": "unknown-status"},
                "junk",
            ],
        }
        db.commit()
    body = signed.get(f"/api/v1/conversations/{conversation}/messages").json()
    assert body["todos"] == [{"content": "残作業", "status": "pending"}]


def test_next_run_carries_todos_into_the_system_prompt(signed, monkeypatch):
    monkeypatch.setattr(routes, "launch", lambda _: None)
    model_id = create_model()
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    with SessionLocal() as db:
        row = db.get(Conversation, conversation)
        row.data = {
            **row.data,
            "todos": [
                {"content": "phase:setup", "status": "completed"},
                {"content": "edit", "status": "in_progress"},
                {"content": "verify", "status": "pending"},
            ],
        }
        db.commit()
    response = signed.post(
        f"/api/v1/conversations/{conversation}/messages",
        json={"model_id": model_id, "content": "continue", "mode": "agent"},
        headers={"Idempotency-Key": conversation},
    )
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        run = db.get(Run, response.json()["run_id"])
        system = run.data["history"][0]["content"]
    assert "Todo list carried over" in system
    assert "- [x] phase:setup" in system
    assert "- [~] edit" in system
    assert "- [ ] verify" in system


def test_chat_mode_prompt_does_not_receive_carried_todos(signed, monkeypatch):
    monkeypatch.setattr(routes, "launch", lambda _: None)
    model_id = create_model()
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    with SessionLocal() as db:
        row = db.get(Conversation, conversation)
        row.data = {**row.data, "todos": [{"content": "残", "status": "pending"}]}
        db.commit()
    response = signed.post(
        f"/api/v1/conversations/{conversation}/messages",
        json={"model_id": model_id, "content": "hello", "mode": "chat"},
        headers={"Idempotency-Key": conversation},
    )
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        run = db.get(Run, response.json()["run_id"])
        system = run.data["history"][0]["content"]
    assert "Todo list carried over" not in system
