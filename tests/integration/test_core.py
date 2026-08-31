from sqlalchemy import select
import pytest
from mix_agent.db.models import *
from mix_agent.db.session import SessionLocal
from mix_agent.tools.registry import BUILTINS, permission, fingerprint, call_scope
from mix_agent.runs import engine


def make_run(tool_name="write_file", mode="agent"):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "Test"})
        db.add(conversation)
        db.flush()
        tool = next(t for t in BUILTINS if t["id"] == tool_name)
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key=uid(),
            data={
                "snapshot": {
                    "mode": mode,
                    "model_id": "fake",
                    "provider": {"kind": "compatible"},
                    "tool_ids": [tool_name],
                    "tools": [tool],
                },
                "history": [{"role": "user", "content": "Do the test"}],
            },
        )
        db.add(run)
        db.commit()
        return run.id, tool


class FakeAdapter:
    def __init__(self, *_):
        pass

    async def stream(self, model, history, tools, mode, settings):
        if history[-1]["role"] == "tool":
            yield {"kind": "text", "text": "Done"}
            yield {
                "kind": "response",
                "message": {"role": "assistant", "content": "Done"},
                "tool_calls": [],
            }
        else:
            name = tools[0]["model_name"]
            yield {
                "kind": "response",
                "message": {"role": "assistant", "content": "I will use a tool"},
                "tool_calls": [
                    {
                        "id": "c1",
                        "name": name,
                        "arguments": {"path": "hello.txt", "content": "hello"},
                    }
                ],
            }


@pytest.mark.parametrize("mode", ["chat", "thinking", "agent"])
async def test_approval_resume_and_no_double_execution(signed, monkeypatch, mode):
    run_id, _ = make_run(mode=mode)
    monkeypatch.setattr(engine, "Adapter", FakeAdapter)
    executions = []

    async def execute(*args):
        executions.append(args[-1])
        return {"ok": True}

    monkeypatch.setattr(engine, "execute", execute)
    await engine.drive(run_id)
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "waiting_approval"
        approval = db.scalar(select(Approval).where(Approval.run_id == run_id))
        approval.status = "once"
        db.commit()
    assert executions == []
    await engine.drive(run_id)
    await engine.drive(run_id)
    assert len(executions) == 1
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "completed"
        sequence = list(
            db.scalars(
                select(Event.sequence)
                .where(Event.run_id == run_id)
                .order_by(Event.sequence)
            )
        )
        assert sequence == list(range(1, len(sequence) + 1))


@pytest.mark.parametrize("mode", ["chat", "thinking", "agent"])
async def test_denied_tool_never_executes(signed, monkeypatch, mode):
    run_id, tool = make_run(mode=mode)
    monkeypatch.setattr(engine, "Adapter", FakeAdapter)

    async def forbidden(*args):
        raise AssertionError("Must not execute")

    monkeypatch.setattr(engine, "execute", forbidden)
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        db.add(
            Permission(
                owner_id=run.owner_id,
                data={
                    "agent_id": "",
                    "tool_id": tool["id"],
                    "tool_version": fingerprint(tool),
                    "scope": call_scope(tool, {}),
                    "permission": "deny",
                },
            )
        )
        db.commit()
    await engine.drive(run_id)
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "completed"
        call = db.scalar(select(ToolCall))
        assert "denied" in call.data["result"]["error"]


def test_memory_revision_restore(signed):
    row = signed.post("/api/v1/memories", json={"content": "日本語を好む"}).json()
    key = row["id"]
    assert (
        signed.patch(
            "/api/v1/memories/" + key, json={"content": "日本語で簡潔に"}
        ).status_code
        == 200
    )
    revisions = signed.get("/api/v1/memories/" + key + "/revisions").json()
    assert revisions[0]["data"]["previous"]["content"] == "日本語を好む"
    assert signed.delete("/api/v1/memories/" + key).status_code == 200
    assert (
        signed.post(
            "/api/v1/memories/" + key + "/restore/" + revisions[0]["id"]
        ).status_code
        == 200
    )
    rows = signed.get("/api/v1/memories?q=日本語").json()
    assert rows[0]["data"]["content"] == "日本語を好む"


def test_skill_revision_restore(signed):
    row = signed.post(
        "/api/v1/skills",
        json={"name": "Release", "description": "verify", "content": "run tests"},
    ).json()
    key = row["id"]
    assert signed.patch(
        "/api/v1/skills/" + key,
        json={"name": "Release", "description": "verify", "content": "run tests then build", "enabled": True},
    ).status_code == 200
    revisions = signed.get("/api/v1/skills/" + key + "/revisions").json()
    assert revisions[0]["data"]["previous"]["content"] == "run tests"
    assert signed.delete("/api/v1/skills/" + key).status_code == 200
    assert signed.post("/api/v1/skills/" + key + "/restore/" + revisions[0]["id"]).status_code == 200
    assert signed.get("/api/v1/skills?q=Release").json()[0]["data"]["content"] == "run tests"


def test_model_override_and_provider_secret(signed):
    response = signed.post(
        "/api/v1/providers",
        json={
            "name": "Local",
            "kind": "compatible",
            "base_url": "http://localhost:9999/v1",
            "allow_private": True,
            "api_key": "test-key-should-not-leak",
        },
    )
    assert response.status_code == 200, response.text
    provider = response.json()
    assert "test-key-should-not-leak" not in response.text
    assert provider["data"]["has_secret_id"]
    row = signed.post(
        "/api/v1/models", json={"provider_id": provider["id"], "model_id": "model"}
    ).json()
    assert (
        signed.patch(
            "/api/v1/models/" + row["id"], json={"overrides": {"tools": True}}
        ).status_code
        == 200
    )
    assert signed.get("/api/v1/models").json()[0]["data"]["overrides"]["tools"] is True


def test_message_requires_idempotency_key(signed):
    conversation = signed.post("/api/v1/conversations", json={}).json()
    response = signed.post(
        "/api/v1/conversations/" + conversation["id"] + "/messages",
        json={"model_id": "not-real", "content": "hello"},
    )
    assert response.status_code == 422


def test_memory_scope_isolation(signed):
    from mix_agent.memory import service

    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        service.change(db, owner, "user preference")
        service.change(db, owner, "agent-only decision", scope="agent:one")
        db.commit()
        assert len(service.search(db, owner, scopes=["user"])) == 1
        assert len(service.search(db, owner, scopes=["user", "agent:one"])) == 2


def test_message_retry_and_saved_selection(signed, monkeypatch):
    from mix_agent.api import routes

    monkeypatch.setattr(routes, "launch", lambda _: None)
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "compatible"})
        db.add(provider)
        db.flush()
        model = Model(
            owner_id=owner, data={"provider_id": provider.id, "model_id": "fake"}
        )
        db.add(model)
        db.commit()
        model_id = model.id
    conversation = signed.post("/api/v1/conversations", json={}).json()
    url = "/api/v1/conversations/" + conversation["id"] + "/messages"
    body = {"model_id": model_id, "content": "hello", "mode": "chat"}
    headers = {"Idempotency-Key": "stable-retry-key"}
    first = signed.post(url, json=body, headers=headers)
    assert first.status_code == 200, first.text
    assert signed.post(url, json=body, headers=headers).json() == first.json()
    assert (
        signed.post(
            url, json={**body, "content": "changed"}, headers=headers
        ).status_code
        == 409
    )
    history = signed.get(url).json()
    assert len(history["messages"]) == len(history["runs"]) == 1
    assert history["selection"]["model_id"] == model_id
