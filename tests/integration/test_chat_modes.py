import asyncio

import pytest
from mix_agent.api import routes
from mix_agent.db.models import Message, Model, Provider, Run, User
from mix_agent.db.session import SessionLocal
from mix_agent.providers.reasoning import reasoning_control, resolve_reasoning
from mix_agent.tools.registry import BUILTINS
from mix_agent.runs.mode_policy import mode_prompt, tool_allowed
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


def test_activity_summary_exposes_only_safe_search_metadata():
    from mix_agent.runs.engine import activity_result, activity_summary, is_user_facing_answer

    assert activity_summary("web_search", {"query": "  AI news  "}) == {
        "icon": "search",
        "label": "Webを検索中",
        "detail": "AI news",
    }
    assert activity_summary("run_terminal", {"command": "cat private.txt"}) == {
        "icon": "terminal",
        "label": "コマンドを実行中",
    }
    assert activity_result("web_search", {
        "results": [
            {"url": "https://example.com/news"},
            {"url": "file:///private.txt"},
        ]
    }) == {
        "icon": "search",
        "label": "1件のWebサイトを検索しました",
        "sources": [{"host": "example.com", "url": "https://example.com/news"}],
        "remaining": 0,
    }
    assert not is_user_facing_answer('{"url":"https://example.com"}')
    assert not is_user_facing_answer('[{"title":"search result"}]')
    assert not is_user_facing_answer("承知しました。確認します。")
    assert not is_user_facing_answer("I'll investigate this now.")
    assert is_user_facing_answer("検索結果を要約しました。")
    assert is_user_facing_answer("確認しますか？必要なら対象を教えてください。")


@pytest.mark.parametrize("mode", ["chat", "thinking"])
def test_default_tools_and_resolved_snapshot(signed, monkeypatch, mode):
    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, mode)
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        snapshot = db.get(Run, response.json()["run_id"]).data["snapshot"]
        assert snapshot["tool_ids"] == [t["id"] for t in BUILTINS if tool_allowed(mode, t)]
        assert snapshot["mode"] == mode
        assert snapshot["mode_prompt"] == mode_prompt(mode)
        assert snapshot["policy"]["planning"] is False
        assert snapshot["reasoning"]["policy"] == (
            "auto" if mode == "chat" else "required"
        )
    assert (
        signed.get("/api/v1/models").json()[0]["data"]["reasoning_control"] == "openai"
    )
    view = signed.get("/api/v1/runs/" + response.json()["run_id"])
    assert view.status_code == 200
    assert view.json()["mode"] == mode
    assert view.json()["policy"]["planning"] is False
    assert view.json()["remaining"]["max_steps"] == view.json()["budget"]["max_steps"]


def test_long_work_snapshots_autonomous_policy_and_tools(signed, monkeypatch):
    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, "agent", acknowledge_unknown_capability=True)
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        run = db.get(Run, response.json()["run_id"])
        snapshot = run.data["snapshot"]
        assert snapshot["policy"]["planning"] is True
        assert snapshot["policy"]["checkpointing"] is True
        assert snapshot["mode_prompt"] == mode_prompt("agent")
        assert "update_plan" in snapshot["tool_ids"]
        assert run.data["checkpoint"]["status"] == "started"


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


@pytest.mark.parametrize("mode", ["chat", "thinking"])
async def test_run_uses_resolved_policy_without_enforcing_step_limit(signed, monkeypatch, mode):
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
                    "content": "done" if len(requests) == 3 else "",
                },
                "tool_calls": [
                    {
                        "id": f"c{len(requests)}",
                        "name": "web_search",
                        "arguments": {"query": "test"},
                    }
                ]
                if tools and len(requests) < 3
                else [],
            }

    async def execute(*args):
        return {"result": "test result"}

    monkeypatch.setattr(engine, "Adapter", RepeatedSearch)
    monkeypatch.setattr(engine, "execute", execute)
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        run.data = {**run.data, "snapshot": {**run.data["snapshot"], "max_steps": 1}}
        db.commit()
    await engine.drive(run_id)
    # The second call is the budget-limited final turn; the third is the
    # existing one-shot user-facing-answer repair because this fake returned empty text.
    assert len(requests) == 3
    assert requests[0][0]
    assert requests[1][0] == requests[2][0] == []
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
                assert '"status": "failed"' in history[-1]["content"]
                assert '"code": "tool_failed"' in history[-1]["content"]
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
async def test_tool_failure_can_lead_to_another_tool_then_a_user_answer(signed, monkeypatch, mode):
    """Tool failures remain model-visible, but intermediate turns never become chat messages."""
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, mode)
    run_id = response.json()["run_id"]
    calls = []

    class AdaptiveAdapter:
        def __init__(self, *_):
            pass

        async def stream(self, model, history, tools, actual_mode, settings):
            calls.append(history)
            if len(calls) == 1:
                yield {"kind": "response", "message": {"role": "assistant", "content": "検索します"}, "tool_calls": [{"id": "first", "name": "web_search", "arguments": {"query": "test"}}]}
            elif len(calls) == 2:
                tool = history[-1]
                assert '"status": "failed"' in tool["content"]
                assert "secret-example" not in tool["content"]
                yield {"kind": "response", "message": {"role": "assistant", "content": ""}, "tool_calls": [{"id": "second", "name": "web_search", "arguments": {"query": "alternative"}}]}
            else:
                yield {"kind": "response", "message": {"role": "assistant", "content": "代替の情報を確認できました。"}, "tool_calls": []}

    async def execute(*args):
        if len(calls) == 1:
            raise RuntimeError("secret-example")
        return {"results": [{"title": "Alternative"}]}

    monkeypatch.setattr(engine, "Adapter", AdaptiveAdapter)
    monkeypatch.setattr(engine, "execute", execute)
    await engine.drive(run_id)
    with SessionLocal() as db:
        messages = list(db.scalars(select(Message).where(Message.conversation_id == db.get(Run, run_id).conversation_id)))
        assert [message.data["content"] for message in messages] == ["test", "代替の情報を確認できました。"]


@pytest.mark.parametrize("mode", ["chat", "thinking"])
async def test_same_tool_and_arguments_are_blocked_after_three_attempts(signed, monkeypatch, mode):
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    run_id = send(signed, monkeypatch, mid, mode).json()["run_id"]
    attempts = 0

    class RepeatingAdapter:
        def __init__(self, *_):
            pass

        async def stream(self, model, history, tools, actual_mode, settings):
            nonlocal attempts
            if history[-1]["role"] == "tool" and '"code": "tool_loop_detected"' in history[-1]["content"]:
                yield {"kind": "response", "message": {"role": "assistant", "content": "同じ検索はこれ以上繰り返さず、ここまでの結果を案内します。"}, "tool_calls": []}
                return
            attempts += 1
            yield {"kind": "response", "message": {"role": "assistant", "content": ""}, "tool_calls": [{"id": str(attempts), "name": "web_search", "arguments": {"query": "same"}}]}

    executions = []

    async def execute(*args):
        executions.append(args[-1])
        return {"results": []}

    monkeypatch.setattr(engine, "Adapter", RepeatingAdapter)
    monkeypatch.setattr(engine, "execute", execute)
    await engine.drive(run_id)
    assert len(executions) == 3
    with SessionLocal() as db:
        history = db.get(Run, run_id).data["history"]
        assert any('"code": "tool_loop_detected"' in item.get("content", "") for item in history)
        assert db.get(Run, run_id).status == "completed"


@pytest.mark.parametrize("mode", ["chat", "thinking"])
async def test_tool_call_limit_gives_model_a_final_turn_without_tools(signed, monkeypatch, mode):
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    agent = signed.post("/api/v1/agents", json={"name": "One tool", "tool_ids": ["web_search"], "max_tool_calls": 1}).json()
    run_id = send(signed, monkeypatch, mid, mode, agent_id=agent["id"]).json()["run_id"]
    # Short modes intentionally ignore an Agent's long-work budget. Override
    # the frozen snapshot here to exercise the engine's final-turn behavior.
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        run.data = {**run.data, "snapshot": {**run.data["snapshot"], "max_tool_calls": 1}}
        db.commit()
    requests = []

    class LimitAdapter:
        def __init__(self, *_):
            pass

        async def stream(self, model, history, tools, actual_mode, settings):
            requests.append((history, tools))
            if len(requests) == 1:
                yield {"kind": "response", "message": {"role": "assistant", "content": ""}, "tool_calls": [
                    {"id": "one", "name": "web_search", "arguments": {"query": "one"}},
                    {"id": "two", "name": "web_search", "arguments": {"query": "two"}},
                ]}
            else:
                yield {"kind": "response", "message": {"role": "assistant", "content": "利用可能な検索結果をもとに回答します。"}, "tool_calls": []}

    async def execute(*args):
        return {"results": []}

    monkeypatch.setattr(engine, "Adapter", LimitAdapter)
    monkeypatch.setattr(engine, "execute", execute)
    await engine.drive(run_id)
    assert len(requests) == 2
    assert requests[1][1] == []
    assert "tool-call limit" in requests[1][0][-1]["content"]
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        assert run.status == "completed", run.data.get("reason")
        assert run.data["tool_count"] == 1
        assert any('"code": "tool_call_limit_reached"' in item.get("content", "") for item in run.data["history"])


@pytest.mark.parametrize("mode", ["chat", "thinking"])
async def test_empty_or_url_json_final_turn_is_repaired_before_persisting(signed, monkeypatch, mode):
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, mode)
    run_id = response.json()["run_id"]
    requests = []

    class EmptyAnswerAdapter:
        def __init__(self, *_):
            pass

        async def stream(self, model, history, tools, actual_mode, settings):
            requests.append(history)
            if len(requests) == 1:
                yield {"kind": "response", "message": {"role": "assistant", "content": '{"url":"https://example.com"}'}, "tool_calls": []}
            else:
                assert "did not provide a user-facing answer" in history[-1]["content"]
                yield {"kind": "response", "message": {"role": "assistant", "content": "明日の天気を確認するには地域を指定してください。"}, "tool_calls": []}

    monkeypatch.setattr(engine, "Adapter", EmptyAnswerAdapter)
    await engine.drive(run_id)
    with SessionLocal() as db:
        messages = list(db.scalars(select(Message).where(Message.conversation_id == db.get(Run, run_id).conversation_id)))
        assert [message.data["content"] for message in messages] == ["test", "明日の天気を確認するには地域を指定してください。"]


async def test_repeated_empty_final_turn_uses_safe_fallback(signed, monkeypatch):
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid)
    run_id = response.json()["run_id"]

    class EmptyAnswerAdapter:
        def __init__(self, *_):
            pass

        async def stream(self, *_):
            yield {"kind": "response", "message": {"role": "assistant", "content": ""}, "tool_calls": []}

    monkeypatch.setattr(engine, "Adapter", EmptyAnswerAdapter)
    await engine.drive(run_id)
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        messages = list(db.scalars(select(Message).where(Message.conversation_id == run.conversation_id)))
        assert run.status == "completed"
        assert len(messages) == 2
        assert "ユーザー向けの本文を生成できませんでした" in messages[-1].data["content"]


@pytest.mark.parametrize("mode", ["chat", "thinking", "agent"])
async def test_progress_promise_does_not_stop_the_run(signed, monkeypatch, mode):
    from mix_agent.runs import engine

    mid = create_model(caps={"reasoning": True, "tools": True})
    response = send(signed, monkeypatch, mid, mode)
    run_id = response.json()["run_id"]
    requests = []

    class ProgressThenAnswerAdapter:
        def __init__(self, *_):
            pass

        async def stream(self, model, history, tools, actual_mode, settings):
            requests.append(history)
            if len(requests) == 1:
                yield {"kind": "text", "text": "承知しました。確認します。"}
                yield {"kind": "response", "message": {"role": "assistant", "content": "承知しました。確認します。"}, "tool_calls": []}
            else:
                assert "does not complete the user's request" in history[-1]["content"]
                yield {"kind": "response", "message": {"role": "assistant", "content": "確認が完了し、結果をまとめました。"}, "tool_calls": []}

    monkeypatch.setattr(engine, "Adapter", ProgressThenAnswerAdapter)
    await engine.drive(run_id)
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        messages = list(db.scalars(select(Message).where(Message.conversation_id == run.conversation_id)))
        assert run.status == "completed"
        assert len(requests) == 2
        assert [message.data["content"] for message in messages] == ["test", "確認が完了し、結果をまとめました。"]


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
