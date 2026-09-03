import asyncio
from sqlalchemy import select
import pytest
from mix_agent.db.models import *
from mix_agent.db.session import SessionLocal
from mix_agent.tools.registry import BUILTINS, permission, fingerprint, call_scope
from mix_agent.tools.execute import save_text_artifact
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


def test_text_artifact_validation_and_metadata(tmp_path, monkeypatch):
    from mix_agent import config

    monkeypatch.setattr(config, "ARTIFACTS", tmp_path)
    with SessionLocal() as db:
        owner = uid()
        artifact = save_text_artifact(db, owner, "watermark-poster.html", "text/html", "<h1>Hello</h1>")
        db.commit()
        assert artifact["name"] == "watermark-poster.html"
        assert artifact["mime"] == "text/html"
        assert artifact["size"] == len("<h1>Hello</h1>".encode())
        assert (tmp_path / artifact["artifact_id"]).read_text() == "<h1>Hello</h1>"
        for name in ("../escape.html", "nested/file.html", "", "bad<script>.html"):
            with pytest.raises(ValueError):
                save_text_artifact(db, owner, name, "text/html", "x")
        with pytest.raises(ValueError):
            save_text_artifact(db, owner, "empty.html", "text/html", "")
        with pytest.raises(ValueError):
            save_text_artifact(db, owner, "large.html", "text/html", "x" * (1024 * 1024 + 1))


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


async def test_safe_tool_calls_share_a_model_turn_and_keep_provider_order(signed, monkeypatch):
    run_id, read_file = make_run(tool_name="read_file")
    search_files = next(tool for tool in BUILTINS if tool["id"] == "search_files")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        snapshot = {
            **run.data["snapshot"],
            "tool_ids": [*run.data["snapshot"]["tool_ids"], "search_files"],
            "tools": [*run.data["snapshot"]["tools"], search_files],
        }
        run.data = {**run.data, "snapshot": snapshot}
        for provider_call_id, tool, arguments in [
            ("first", read_file, {"path": "first.txt"}),
            ("second", search_files, {"query": "second"}),
        ]:
            db.add(
                ToolCall(
                    owner_id=run.owner_id,
                    run_id=run.id,
                    data={
                        "tool_id": tool["id"], "tool_version": fingerprint(tool),
                        "provider_call_id": provider_call_id,
                        "name": tool["model_name"], "arguments": arguments,
                    },
                )
            )
        db.commit()

    class CompleteAfterTools:
        def __init__(self, *_):
            pass

        async def stream(self, *_):
            yield {"kind": "response", "message": {"role": "assistant", "content": "done"}, "tool_calls": []}

    active = 0
    maximum = 0

    async def delayed_execute(*args):
        nonlocal active, maximum
        active += 1
        maximum = max(maximum, active)
        await asyncio.sleep(0.02)
        active -= 1
        return {"call": args[-1]}

    monkeypatch.setattr(engine, "Adapter", CompleteAfterTools)
    monkeypatch.setattr(engine, "execute", delayed_execute)
    await engine.drive(run_id)
    assert maximum == 2
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert [entry["call_id"] for entry in run.data["history"] if entry["role"] == "tool"] == ["first", "second"]


async def test_serial_tool_calls_do_not_overlap(signed, monkeypatch):
    run_id, read_file = make_run(tool_name="read_file")
    update_plan = next(tool for tool in BUILTINS if tool["id"] == "update_plan")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        snapshot = {
            **run.data["snapshot"],
            "tool_ids": [*run.data["snapshot"]["tool_ids"], "update_plan"],
            "tools": [*run.data["snapshot"]["tools"], update_plan],
        }
        run.data = {**run.data, "snapshot": snapshot}
        for provider_call_id, tool, arguments in [
            ("read", read_file, {"path": "read.txt"}),
            ("plan", update_plan, {"steps": ["one"]}),
        ]:
            db.add(
                ToolCall(
                    owner_id=run.owner_id,
                    run_id=run.id,
                    data={
                        "tool_id": tool["id"], "tool_version": fingerprint(tool),
                        "provider_call_id": provider_call_id,
                        "name": tool["model_name"], "arguments": arguments,
                    },
                )
            )
        db.commit()

    class CompleteAfterTools:
        def __init__(self, *_):
            pass

        async def stream(self, *_):
            yield {"kind": "response", "message": {"role": "assistant", "content": "done"}, "tool_calls": []}

    active = 0
    maximum = 0

    async def delayed_execute(*_):
        nonlocal active, maximum
        active += 1
        maximum = max(maximum, active)
        await asyncio.sleep(0.01)
        active -= 1
        return {"ok": True}

    monkeypatch.setattr(engine, "Adapter", CompleteAfterTools)
    monkeypatch.setattr(engine, "execute", delayed_execute)
    await engine.drive(run_id)
    assert maximum == 1


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
    deleted = signed.get("/api/v1/memories?q=日本語").json()
    assert deleted[0]["data"]["deleted"] is True
    assert (
        signed.post(
            "/api/v1/memories/" + key + "/restore/" + revisions[0]["id"]
        ).status_code
        == 200
    )
    rows = signed.get("/api/v1/memories?q=日本語").json()
    assert rows[0]["data"]["content"] == "日本語を好む"


def test_memory_recall_ranks_pinned_relevant_entries_and_blocks_secrets(signed):
    pinned = signed.post(
        "/api/v1/memories",
        json={"content": "回答は日本語で簡潔にする", "importance": 5, "pinned": True},
    )
    assert pinned.status_code == 200
    duplicate = signed.post(
        "/api/v1/memories",
        json={"content": "回答は日本語で簡潔にする", "importance": 1},
    )
    assert duplicate.status_code == 200
    assert duplicate.json()["deduplicated"] is True
    signed.post("/api/v1/memories", json={"content": "Pythonでは型ヒントを使う", "importance": 3})
    rows = signed.get("/api/v1/memories?q=日本語の回答").json()
    assert rows[0]["data"]["pinned"] is True
    assert "category" not in rows[0]["data"]
    assert rows[0]["data"]["lifecycle_state"] == "established"
    assert rows[0]["data"]["strength"] > 0.8
    rejected = signed.post("/api/v1/memories", json={"content": "api_key=not-for-memory"})
    assert rejected.status_code == 422


def test_memory_explicit_capture_only_accepts_clear_requests():
    from mix_agent.memory.service import explicit_candidate

    assert explicit_candidate("覚えて: 回答は日本語で簡潔に") == "回答は日本語で簡潔に"
    assert explicit_candidate("回答は日本語で簡潔にするを覚えておいて") == "回答は日本語で簡潔にする"
    assert explicit_candidate("これを覚えて") is None
    assert explicit_candidate("覚えて: api_key=private") is None


def test_web_search_is_allowed_by_default_but_explicit_rules_win(signed):
    run_id, tool = make_run(tool_name="web_search", mode="chat")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        arguments = {"query": "current technology news"}
        assert permission(db, run, tool, arguments) == "allow"

        db.add(
            Permission(
                owner_id=run.owner_id,
                data={
                    "agent_id": "",
                    "tool_id": tool["id"],
                    "tool_version": fingerprint(tool),
                    "scope": call_scope(tool, arguments),
                    "permission": "ask",
                },
            )
        )
        db.commit()
        assert permission(db, run, tool, arguments) == "ask"


def test_permission_rule_api_persists_real_scope_and_legacy_empty_scope_matches(signed):
    run_id, tool = make_run(tool_name="run_terminal", mode="chat")
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        response = signed.post(
            "/api/v1/permission-rules",
            json={"tool_id": tool["id"], "permission": "deny"},
        )
        assert response.status_code == 200
        assert response.json()["data"]["scope"] == {"workspace": "/workspace"}
        assert permission(db, run, tool, {"command": "echo hi"}) == "deny"
        legacy = next(
            r for r in db.scalars(select(Permission)) if r.data.get("tool_id") == tool["id"]
        )
        legacy.data = {**legacy.data, "scope": {}}
        db.commit()
        assert permission(db, run, tool, {"command": "echo hi"}) == "deny"


def test_browser_setup_bundle_and_deferred_install_status(signed, monkeypatch):
    from mix_agent.api import routes

    async def provisioner(kind, path, payload, timeout=120):
        assert kind == "browser-provisioner"
        if path == "/install":
            return {"status": "installing", "failure": None}
        return {"status": "ready", "failure": None}

    monkeypatch.setattr(routes, "runner_request", provisioner)
    response = signed.post("/api/v1/browser/enable")
    assert response.status_code == 200
    assert {"browser_open", "browser_read", "browser_click", "browser_type"} <= set(response.json()["tool_ids"])
    response = signed.post("/api/v1/browser/install")
    assert response.status_code == 200
    assert response.json()["status"] == "installing"
    response = signed.get("/api/v1/settings")
    assert response.status_code == 200
    assert response.json()["data"]["browser_install_status"] == "ready"
    with SessionLocal() as db:
        rules = list(db.scalars(select(Permission)))
        browser = [row.data for row in rules if row.data.get("tool_id", "").startswith("browser_")]
        assert browser and all(row["permission"] == "allow" for row in browser)

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


def test_memory_scope_is_a_soft_retrieval_signal(signed):
    from mix_agent.memory import service

    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        service.change(db, owner, "user preference", confidence=.9)
        service.change(db, owner, "agent-only decision", scope="agent:one", confidence=.9)
        db.commit()
        user_ranked = service.search(db, owner, scopes=["user"])
        agent_ranked = service.search(db, owner, scopes=["agent:one"])
        assert {row["content"] for row in user_ranked} == {"user preference", "agent-only decision"}
        assert user_ranked[0]["content"] == "user preference"
        assert agent_ranked[0]["content"] == "agent-only decision"


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


def test_message_uses_auto_then_conversation_selection_when_omitted(signed, monkeypatch):
    from mix_agent.api import routes

    monkeypatch.setattr(routes, "launch", lambda _: None)
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "compatible"})
        db.add(provider)
        db.flush()
        model = Model(owner_id=owner, data={"provider_id": provider.id, "model_id": "fake"})
        db.add(model)
        db.commit()
        model_id = model.id
    assert signed.put("/api/v1/settings", json={
        "default_model_id": "auto", "auto_model_ids": [model_id],
    }).status_code == 200
    conversation = signed.post("/api/v1/conversations", json={}).json()
    url = "/api/v1/conversations/" + conversation["id"] + "/messages"
    first = signed.post(url, json={"content": "hello"}, headers={"Idempotency-Key": "auto-first"})
    assert first.status_code == 200, first.text
    with SessionLocal() as db:
        db.get(Run, first.json()["run_id"]).status = "completed"
        db.commit()
    second = signed.post(url, json={"content": "again"}, headers={"Idempotency-Key": "auto-second"})
    assert second.status_code == 200, second.text
    history = signed.get(url).json()
    assert history["selection"] == {"model_id": "auto", "agent_id": "", "mode": "chat"}
    with SessionLocal() as db:
        assert db.get(Run, first.json()["run_id"]).data["snapshot"]["requested_model_id"] == "auto"
        assert db.get(Run, second.json()["run_id"]).data["snapshot"]["requested_model_id"] == "auto"
