import asyncio

import httpx
from sqlalchemy import select

from mix_agent.api import routes
from mix_agent.db.models import (
    Conversation,
    Feedback,
    Message,
    Model,
    Provider,
    Run,
    Settings,
    User,
)
from mix_agent.db.session import SessionLocal
from mix_agent.providers.adapters import ProviderContextLimitError, is_nvidia_nim_function_not_found
from mix_agent.routing import select_auto_model
from mix_agent.runs import engine


def make_model(caps, context=20000, model_id="auto-test", provider_data=None):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data=provider_data or {"kind": "openai"})
        db.add(provider)
        db.flush()
        model = Model(owner_id=owner, data={"provider_id": provider.id, "model_id": model_id,
                                             "capabilities": caps, "context_window": context})
        db.add(model)
        db.commit()
        return model.id


def test_auto_settings_and_capability_filters(signed, monkeypatch):
    plain = make_model({"tools": True}, model_id="plain")
    vision = make_model({"tools": True, "vision": True}, model_id="vision")
    assert signed.put("/api/v1/settings", json={"default_model_id": "auto", "auto_model_ids": [plain, vision],
                                                  "allowed_domains": []}).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    response = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat"}, headers={"Idempotency-Key": conversation})
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        run = db.get(Run, response.json()["run_id"])
        assert run.data["requested_model_id"] == "auto"
        assert run.data["snapshot"]["model_record_id"] in (plain, vision)
        assert run.data["auto_selection"]["candidate_count"] == 2
        owner = db.scalar(select(User.id))
        selected, details = select_auto_model(db, owner, [plain, vision], "look", "chat", ["image/png"], False,
                                              ["look"], 10, "a-unique-request")
        assert selected.id == vision
        assert details["candidate_count"] == 1


def test_auto_retry_count_defaults_to_three_for_legacy_settings(signed):
    with SessionLocal() as db:
        settings = db.get(Settings, "settings")
        settings.data = {"setup_complete": False, "allowed_domains": []}
        db.commit()
    assert signed.get("/api/v1/settings").json()["data"]["auto_retry_count"] == 3


def test_unknown_context_model_is_usable_by_auto_without_an_override(signed, monkeypatch):
    unknown = make_model({}, context=None, model_id="unknown")
    assert signed.put("/api/v1/settings", json={"default_model_id": "auto", "auto_model_ids": [unknown],
                                                  "allowed_domains": []}).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    response = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat"}, headers={"Idempotency-Key": conversation})
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        run = db.get(Run, response.json()["run_id"])
        assert run.data["auto_selection"]["candidate_ids"] == [unknown]
        assert run.data["auto_selection"]["context_usage"][unknown]["window"] is None


def test_context_usage_penalizes_known_models_above_half_capacity(signed):
    tight = make_model({}, context=100, model_id="tight")
    roomy = make_model({}, context=1000, model_id="roomy")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        selected, details = select_auto_model(
            db, owner, [tight, roomy], "x" * 120, "chat", [], False, ["x" * 120], 10, "context-usage"
        )
    assert selected.id == roomy
    assert details["context_usage"][tight]["usage_ratio"] > 0.5
    assert details["context_usage"][tight]["penalty"] > 0


def test_auto_retries_once_after_pre_output_context_error(signed, monkeypatch):
    first = make_model({}, context=None, model_id="first")
    second = make_model({}, context=None, model_id="second")
    assert signed.put("/api/v1/settings", json={"default_model_id": "auto", "auto_model_ids": [first, second],
                                                  "allowed_domains": []}).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat"}, headers={"Idempotency-Key": "context-retry"})
    assert queued.status_code == 200, queued.text
    attempts = []

    class FakeAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            attempts.append(model)
            if len(attempts) == 1:
                raise ProviderContextLimitError("context limit")
            yield {"kind": "response", "message": {"role": "assistant", "content": "ok"}, "tool_calls": []}

    monkeypatch.setattr(engine, "Adapter", FakeAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        run = db.get(Run, queued.json()["run_id"])
        message = db.scalar(select(Message).where(Message.conversation_id == conversation, Message.data["role"].as_string() == "assistant"))
        assert run.status == "completed"
        assert len(attempts) == 2
        assert len(run.data["auto_selection"]["attempts"]) == 2
        assert message.data["auto_selection"]["model_id"] == attempts[-1]


def test_auto_retries_transient_provider_error_but_not_auth_error(signed, monkeypatch):
    first = make_model({}, context=None, model_id="transient-first")
    second = make_model({}, context=None, model_id="transient-second")
    assert signed.put("/api/v1/settings", json={"default_model_id": "auto", "auto_model_ids": [first, second],
                                                  "allowed_domains": []}).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat"}, headers={"Idempotency-Key": "transient-retry"})
    attempts = []

    class FakeAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            attempts.append(model)
            response = httpx.Response(503, request=httpx.Request("POST", "https://provider.test"))
            if len(attempts) == 1:
                raise httpx.HTTPStatusError("temporary", request=response.request, response=response)
            yield {"kind": "response", "message": {"role": "assistant", "content": "ok"}, "tool_calls": []}

    monkeypatch.setattr(engine, "Adapter", FakeAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        run = db.get(Run, queued.json()["run_id"])
        assert run.status == "completed"
        assert len(attempts) == 2
        assert run.data["auto_selection"]["attempts"][0]["outcome"] == "provider_error"

    attempts.clear()
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat"}, headers={"Idempotency-Key": "auth-no-retry"})

    class AuthFailAdapter(FakeAdapter):
        async def stream(self, model, history, tools, mode, settings):
            attempts.append(model)
            response = httpx.Response(401, request=httpx.Request("POST", "https://provider.test"))
            raise httpx.HTTPStatusError("unauthorized", request=response.request, response=response)
            yield

    monkeypatch.setattr(engine, "Adapter", AuthFailAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        run = db.get(Run, queued.json()["run_id"])
        assert run.status == "failed"
        assert len(attempts) == 1


def test_auto_retry_count_uses_each_candidate_once_and_is_snapshotted(signed, monkeypatch):
    model_ids = [make_model({}, context=None, model_id=f"retry-{index}") for index in range(4)]
    assert signed.put("/api/v1/settings", json={
        "default_model_id": "auto", "auto_model_ids": model_ids, "auto_retry_count": 3, "allowed_domains": [],
    }).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat",
    }, headers={"Idempotency-Key": "three-auto-retries"})
    assert queued.status_code == 200, queued.text
    assert signed.put("/api/v1/settings", json={
        "default_model_id": "auto", "auto_model_ids": model_ids, "auto_retry_count": 0, "allowed_domains": [],
    }).status_code == 200
    attempts = []

    class FakeAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            attempts.append(model)
            if len(attempts) <= 3:
                response = httpx.Response(503, request=httpx.Request("POST", "https://provider.test"))
                raise httpx.HTTPStatusError("temporary", request=response.request, response=response)
            yield {"kind": "response", "message": {"role": "assistant", "content": "ok"}, "tool_calls": []}

    monkeypatch.setattr(engine, "Adapter", FakeAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        run = db.get(Run, queued.json()["run_id"])
        assert run.status == "completed"
        assert run.data["snapshot"]["auto_retry_count"] == 3
        assert len(attempts) == len(set(attempts)) == 4
        retries = [item for item in run.data["auto_selection"]["attempts"] if item["outcome"] == "retry"]
        assert [item["retry_number"] for item in retries] == [1, 2, 3]


def test_auto_does_not_retry_not_found_or_when_retry_count_is_zero(signed, monkeypatch):
    first = make_model({}, context=None, model_id="not-found-first")
    second = make_model({}, context=None, model_id="not-found-second")
    assert signed.put("/api/v1/settings", json={
        "default_model_id": "auto", "auto_model_ids": [first, second], "auto_retry_count": 3, "allowed_domains": [],
    }).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat",
    }, headers={"Idempotency-Key": "not-found-no-retry"})
    attempts = []

    class NotFoundAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            attempts.append(model)
            error = type("NotFoundError", (Exception,), {})()
            raise error
            yield

    monkeypatch.setattr(engine, "Adapter", NotFoundAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        run = db.get(Run, queued.json()["run_id"])
        assert run.status == "failed"
        assert len(attempts) == 1
        assert "モデルがProviderに見つかりません" in run.data["reason"]

    assert signed.put("/api/v1/settings", json={
        "default_model_id": "auto", "auto_model_ids": [first, second], "auto_retry_count": 0, "allowed_domains": [],
    }).status_code == 200
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat",
    }, headers={"Idempotency-Key": "zero-retry"})
    attempts.clear()

    class TemporaryFailureAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            attempts.append(model)
            response = httpx.Response(503, request=httpx.Request("POST", "https://provider.test"))
            raise httpx.HTTPStatusError("temporary", request=response.request, response=response)
            yield

    monkeypatch.setattr(engine, "Adapter", TemporaryFailureAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        run = db.get(Run, queued.json()["run_id"])
        assert run.status == "failed"
        assert run.data["snapshot"]["auto_retry_count"] == 0
        assert len(attempts) == 1


def test_auto_retries_nvidia_nim_function_not_found_and_reports_entitlement_failure(signed, monkeypatch):
    nvidia = {"kind": "compatible", "preset_id": "nvidia-nim"}
    first = make_model({}, context=None, model_id="nim-first", provider_data=nvidia)
    second = make_model({}, context=None, model_id="nim-second", provider_data=nvidia)
    assert signed.put("/api/v1/settings", json={
        "default_model_id": "auto", "auto_model_ids": [first, second], "auto_retry_count": 3, "allowed_domains": [],
    }).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat",
    }, headers={"Idempotency-Key": "nvidia-nim-function-not-found"})
    attempts = []

    class NIMUnavailableAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            attempts.append(model)
            response = httpx.Response(
                404,
                json={"detail": "Function 'example' not found for account 'example'"},
                request=httpx.Request("POST", "https://integrate.api.nvidia.com/v1/chat/completions"),
            )
            raise httpx.HTTPStatusError("not found", request=response.request, response=response)
            yield

    monkeypatch.setattr(engine, "Adapter", NIMUnavailableAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        run = db.get(Run, queued.json()["run_id"])
        assert run.status == "failed"
        assert len(attempts) == len(set(attempts)) == 2
        assert [item["outcome"] for item in run.data["auto_selection"]["attempts"]] == ["provider_error", "retry"]
        assert "NVIDIA NIM" in run.data["reason"]
        assert "利用権限" in run.data["reason"]


def test_nvidia_nim_function_not_found_does_not_retry_for_manual_selection(signed, monkeypatch):
    nvidia = {"kind": "compatible", "preset_id": "nvidia-nim"}
    model = make_model({}, context=None, model_id="nim-manual", provider_data=nvidia)
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": model, "content": "hello", "mode": "chat",
    }, headers={"Idempotency-Key": "nvidia-nim-manual-not-found"})
    attempts = []

    class NIMUnavailableAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, selected, history, tools, mode, settings):
            attempts.append(selected)
            response = httpx.Response(
                404,
                json={"detail": "Function 'example' not found for account 'example'"},
                request=httpx.Request("POST", "https://integrate.api.nvidia.com/v1/chat/completions"),
            )
            raise httpx.HTTPStatusError("not found", request=response.request, response=response)
            yield

    monkeypatch.setattr(engine, "Adapter", NIMUnavailableAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        run = db.get(Run, queued.json()["run_id"])
        assert run.status == "failed"
        assert attempts == ["nim-manual"]
        assert "NVIDIA NIM" in run.data["reason"]


def test_nvidia_nim_function_not_found_classification_is_limited_to_hosted_404():
    response = httpx.Response(
        404,
        json={"detail": "Function 'example' not found for account 'example'"},
        request=httpx.Request("POST", "https://integrate.api.nvidia.com/v1/chat/completions"),
    )
    unavailable = httpx.HTTPStatusError("not found", request=response.request, response=response)
    assert is_nvidia_nim_function_not_found({"preset_id": "nvidia-nim"}, unavailable)
    assert not is_nvidia_nim_function_not_found({"preset_id": "openai"}, unavailable)

    unauthorized = httpx.Response(401, request=response.request)
    auth_error = httpx.HTTPStatusError("unauthorized", request=response.request, response=unauthorized)
    assert not is_nvidia_nim_function_not_found({"preset_id": "nvidia-nim"}, auth_error)

    missing = httpx.Response(404, json={"detail": "model not found"}, request=response.request)
    missing_error = httpx.HTTPStatusError("not found", request=response.request, response=missing)
    assert not is_nvidia_nim_function_not_found({"preset_id": "nvidia-nim"}, missing_error)


def test_ucb_feedback_is_profile_scoped_and_single_rating_is_smoothed(signed):
    first = make_model({}, model_id="first")
    second = make_model({}, model_id="second")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "ratings"})
        db.add(conversation)
        db.flush()
        for _ in range(30):
            message = Message(owner_id=owner, conversation_id=conversation.id, data={"role": "assistant", "content": "x"})
            db.add(message)
            db.flush()
            db.add(Feedback(owner_id=owner, message_id=message.id, data={"model_id": first, "profile": "coding", "value": "up"}))
        db.commit()
        selected, _ = select_auto_model(db, owner, [first, second], "code", "chat", [], False, ["code"], 10, "b-request")
        # A well-tested model wins over the exploration bonus; a different profile does not use these ratings.
        assert selected.id == first
        selected, _ = select_auto_model(db, owner, [first, second], "hello", "chat", [], False, ["hello"], 10, "c-request")
        assert selected.id in (first, second)


def test_feedback_replaces_or_clears_only_auto_answers(signed):
    model_id = make_model({}, model_id="rated")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "test"})
        db.add(conversation)
        db.flush()
        message = Message(owner_id=owner, conversation_id=conversation.id, data={"role": "assistant", "content": "x",
            "auto_selection": {"model_record_id": model_id, "profile": "general"}})
        db.add(message)
        db.commit()
        message_id = message.id
    assert signed.put(f"/api/v1/messages/{message_id}/feedback", json={"value": "up"}).json()["value"] == "up"
    assert signed.put(f"/api/v1/messages/{message_id}/feedback", json={"value": "down"}).json()["value"] == "down"
    assert signed.put(f"/api/v1/messages/{message_id}/feedback", json={"value": None}).json()["value"] is None
    with SessionLocal() as db:
        assert db.scalar(select(Feedback).where(Feedback.message_id == message_id)) is None
