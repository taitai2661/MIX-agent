import asyncio

import pytest
from mix_agent.api import routes
from mix_agent.db.models import Model, Provider, Run, User
from mix_agent.db.session import SessionLocal
from mix_agent.providers.reasoning import reasoning_control, resolve_reasoning
from mix_agent.tools.registry import BUILTINS
from sqlalchemy import select


def create_model(kind="openai", caps=None, model_id="test-reasoning"):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": kind})
        db.add(provider)
        db.flush()
        model = Model(
            owner_id=owner,
            data={
                "provider_id": provider.id,
                "model_id": model_id,
                "capabilities": caps or {},
            },
        )
        db.add(model)
        db.commit()
        return model.id


def send(signed, monkeypatch, model_id, mode="chat", **kwargs):
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    return signed.post(
        f"/api/v1/conversations/{conversation}/messages",
        json={"model_id": model_id, "content": "test", "mode": mode, **kwargs},
        headers={"Idempotency-Key": conversation},
    )


@pytest.mark.parametrize("mode", ["chat", "thinking"])
def test_default_tools_and_resolved_snapshot(signed, monkeypatch, mode):
    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, mode)
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        snapshot = db.get(Run, response.json()["run_id"]).data["snapshot"]
        assert snapshot["tool_ids"] == [t["id"] for t in BUILTINS]
        assert snapshot["reasoning"]["policy"] == (
            "auto" if mode == "chat" else "required"
        )
    assert (
        signed.get("/api/v1/models").json()[0]["data"]["reasoning_control"] == "openai"
    )


@pytest.mark.parametrize("tool_ids", [[], ["web_search"]])
def test_preset_tool_restrictions(signed, monkeypatch, tool_ids):
    mid = create_model(caps={"reasoning": True, "tools": True})
    preset = signed.post(
        "/api/v1/agents", json={"name": "Limited", "tool_ids": tool_ids}
    ).json()
    response = send(signed, monkeypatch, mid, agent_id=preset["id"])
    assert response.status_code == 200
    with SessionLocal() as db:
        assert (
            db.get(Run, response.json()["run_id"]).data["snapshot"]["tool_ids"]
            == tool_ids
        )


@pytest.mark.parametrize("caps", [{}, {"reasoning": False, "tools": False}])
def test_chat_keeps_plain_conversation_available(signed, monkeypatch, caps):
    mid = create_model(caps=caps)
    response = send(signed, monkeypatch, mid)
    assert response.status_code == 200
    with SessionLocal() as db:
        snapshot = db.get(Run, response.json()["run_id"]).data["snapshot"]
        assert snapshot["tools"] == []
        assert snapshot["reasoning"]["request"] == {}


@pytest.mark.parametrize("kind,caps", [("openai", {}), ("compatible", {"reasoning": True}), ("ollama", {})])
def test_thinking_falls_back_to_ordinary_inference(signed, monkeypatch, kind, caps):
    mid = create_model(kind, {**caps, "tools": True})
    response = send(signed, monkeypatch, mid, "thinking")
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        snapshot = db.get(Run, response.json()["run_id"]).data["snapshot"]
        assert snapshot["reasoning"] == {"policy": "tool_assisted", "request": {}, "summary": False}


def test_thinking_probes_unknown_tools_once_and_persists_result(signed, monkeypatch):
    mid = create_model("compatible", {})
    calls = []

    class ProbeAdapter:
        def __init__(self, *_):
            pass

        async def probe_tools(self, _):
            calls.append(True)
            return True

    monkeypatch.setattr(routes, "Adapter", ProbeAdapter)
    first = send(signed, monkeypatch, mid, "thinking")
    second = send(signed, monkeypatch, mid, "thinking")
    assert first.status_code == second.status_code == 200
    assert len(calls) == 1
    with SessionLocal() as db:
        model = db.get(Model, mid)
        assert model.data["tool_probe"]["status"] == "supported"
        assert db.get(Run, first.json()["run_id"]).data["snapshot"]["tools"]


def test_unknown_tool_probe_failure_is_not_retried_until_manual_check(signed, monkeypatch):
    mid = create_model("compatible", {})
    calls = []

    class ProbeAdapter:
        def __init__(self, *_):
            pass

        async def probe_tools(self, _):
            calls.append(True)
            raise TimeoutError()

    monkeypatch.setattr(routes, "Adapter", ProbeAdapter)
    assert send(signed, monkeypatch, mid, "thinking").status_code == 200
    assert send(signed, monkeypatch, mid, "thinking").status_code == 200
    assert len(calls) == 1
    with SessionLocal() as db:
        model = db.get(Model, mid)
        assert model.data["tool_probe"]["status"] == "unknown"
    response = signed.post(f"/api/v1/models/{mid}/verify-tools")
    assert response.status_code == 200
    assert len(calls) == 2


def test_tool_probe_survives_model_edit_and_provider_sync(signed, monkeypatch):
    mid = create_model("compatible", {})
    with SessionLocal() as db:
        model = db.get(Model, mid)
        provider_id = model.data["provider_id"]
        model.data = {**model.data, "tool_probe": {"status": "supported", "source": "automatic"}}
        db.commit()
    response = signed.patch(f"/api/v1/models/{mid}", json={"overrides": {"vision": True}})
    assert response.status_code == 200

    class SyncAdapter:
        def __init__(self, *_):
            pass

        async def list_models(self):
            return [{"model_id": "test-reasoning", "capabilities": {}, "overrides": {}}]

    monkeypatch.setattr(routes, "Adapter", SyncAdapter)
    response = signed.post(f"/api/v1/providers/{provider_id}/sync-models")
    assert response.status_code == 200
    with SessionLocal() as db:
        assert db.get(Model, mid).data["tool_probe"]["status"] == "supported"


def test_manual_tool_override_wins_over_automatic_probe(signed, monkeypatch):
    mid = create_model("compatible", {})
    with SessionLocal() as db:
        model = db.get(Model, mid)
        model.data = {**model.data, "overrides": {"tools": False}}
        db.commit()

    class ForbiddenProbe:
        def __init__(self, *_):
            raise AssertionError("manual override must skip the probe")

    monkeypatch.setattr(routes, "Adapter", ForbiddenProbe)
    response = send(signed, monkeypatch, mid, "thinking")
    assert response.status_code == 200
    with SessionLocal() as db:
        assert db.get(Run, response.json()["run_id"]).data["snapshot"]["tools"] == []


@pytest.mark.parametrize(
    "kind,model",
    [
        ("openai", "test"),
        ("openrouter", "test"),
        ("anthropic", "claude-sonnet-4-5"),
        ("anthropic", "claude-sonnet-4-6"),
        ("gemini", "gemini-2.5-flash"),
        ("gemini", "gemini-3-flash-preview"),
    ],
)
def test_required_cannot_be_disabled_by_preset(kind, model):
    with pytest.raises(ValueError):
        resolve_reasoning(
            kind, model, {"reasoning": True}, "thinking", {"reasoning_effort": "none"}
        )
    assert (
        resolve_reasoning(
            kind, model, {"reasoning": True}, "chat", {"reasoning_effort": "none"}
        )["policy"]
        == "auto"
    )


def test_anthropic_budget_and_unknown_model():
    with pytest.raises(ValueError, match="1024"):
        resolve_reasoning(
            "anthropic",
            "claude-sonnet-4-5",
            {"reasoning": True},
            "thinking",
            {"max_output_tokens": 1024},
        )
    assert reasoning_control("anthropic", "unverified-model") is None
    assert reasoning_control("anthropic", "claude-sonnet-4-99") is None


def test_agent_preserves_legacy_policy():
    assert resolve_reasoning("compatible", "test", {}, "agent", {}) == {
        "policy": "legacy"
    }


@pytest.mark.parametrize("mode,limit", [("chat", 3), ("thinking", 5)])
async def test_run_uses_resolved_policy_and_keeps_step_limit(
    signed, monkeypatch, mode, limit
):
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, mode)
    run_id = response.json()["run_id"]
    requests = []

    class RepeatedSearch:
        def __init__(self, *_):
            pass

        async def stream(self, model, history, tools, actual_mode, settings):
            requests.append((tools, settings))
            yield {"kind": "reasoning", "text": "Public summary"}
            yield {
                "kind": "response",
                "message": {
                    "role": "assistant",
                    "content": "done" if not tools else "",
                },
                "tool_calls": [
                    {
                        "id": f"c{len(requests)}",
                        "name": "web_search",
                        "arguments": {"query": "test"},
                    }
                ]
                if tools
                else [],
            }

    async def execute(*args):
        return {"result": "test result"}

    monkeypatch.setattr(engine, "Adapter", RepeatedSearch)
    monkeypatch.setattr(engine, "execute", execute)
    await engine.drive(run_id)
    assert len(requests) == limit
    assert requests[0][0] and not requests[-1][0]
    assert all(
        s["_resolved_reasoning"]["policy"] == ("auto" if mode == "chat" else "required")
        for _, s in requests
    )
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "completed"


@pytest.mark.parametrize("mode", ["chat", "thinking"])
async def test_cancelled_run_never_calls_provider(signed, monkeypatch, mode):
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, mode)
    run_id = response.json()["run_id"]
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        run.status = "cancelled"
        db.commit()

    def forbidden(*args):
        raise AssertionError("Cancelled run must not contact provider")

    monkeypatch.setattr(engine, "Adapter", forbidden)
    await engine.drive(run_id)
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "cancelled"


@pytest.mark.parametrize("mode", ["chat", "thinking"])
async def test_tool_failure_is_reported_back_to_model(signed, monkeypatch, mode):
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, mode)
    run_id = response.json()["run_id"]

    class FailedSearch:
        def __init__(self, *_):
            pass

        async def stream(self, model, history, tools, actual_mode, settings):
            if history[-1]["role"] == "tool":
                assert "Tool failed" in history[-1]["content"]
                assert "secret-example" not in history[-1]["content"]
                yield {
                    "kind": "response",
                    "message": {"role": "assistant", "content": "Search failed"},
                    "tool_calls": [],
                }
            else:
                yield {
                    "kind": "response",
                    "message": {"role": "assistant", "content": ""},
                    "tool_calls": [
                        {
                            "id": "c",
                            "name": "web_search",
                            "arguments": {"query": "test"},
                        }
                    ],
                }

    async def fail(*args):
        raise RuntimeError("secret-example")

    monkeypatch.setattr(engine, "Adapter", FailedSearch)
    monkeypatch.setattr(engine, "execute", fail)
    await engine.drive(run_id)
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "completed"


@pytest.mark.parametrize("mode", ["chat", "thinking"])
async def test_running_stream_can_be_cancelled(signed, monkeypatch, mode):
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, mode)
    run_id = response.json()["run_id"]
    started = asyncio.Event()
    cancels = []

    class WaitingAdapter:
        def __init__(self, *_):
            pass

        async def stream(self, *args):
            started.set()
            await asyncio.Event().wait()
            yield {"kind": "text", "text": "unreachable"}

    async def cancel_runner(kind, path, body, **kwargs):
        cancels.append((kind, path))
        return {}

    monkeypatch.setattr(engine, "Adapter", WaitingAdapter)
    monkeypatch.setattr(engine, "runner_request", cancel_runner)
    task = asyncio.create_task(engine.drive(run_id))
    try:
        await asyncio.wait_for(started.wait(), 3)
    finally:
        task.cancel()
        await task
    assert cancels == [("execution", "/cancel"), ("mcp", "/cancel")]
    with SessionLocal() as db:
        assert db.get(Run, run_id).status == "cancelled"
