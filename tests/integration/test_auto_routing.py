import asyncio
from datetime import timedelta

import httpx
from mix_agent.api import routes
from mix_agent.db.models import (
    AutoReliabilityEvent,
    Conversation,
    Feedback,
    Message,
    Model,
    PerformanceEvent,
    Provider,
    Run,
    Settings,
    User,
)
from mix_agent.db.session import SessionLocal
from mix_agent.providers.adapters import (
    ProviderContextLimitError,
    is_nvidia_nim_chat_incompatible,
    is_nvidia_nim_function_not_found,
)
from mix_agent.reliability import classify_failure, reliability, retry_after, speed
from mix_agent.reliability import record as record_reliability
from mix_agent.routing import routing_profile, select_auto_model
from mix_agent.runs import engine
from sqlalchemy import select


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


def test_auto_excludes_known_special_purpose_models(signed):
    parse = make_model({}, model_id="nvidia/nemotron-parse", provider_data={"kind": "nvidia"})
    chat = make_model({}, model_id="nvidia/llama-3.3-nemotron-super-49b-v1.5", provider_data={"kind": "nvidia"})
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        selected, details = select_auto_model(
            db, owner, [parse, chat], "hello", "chat", [], False, ["hello"], 10, "special-purpose",
        )
        assert selected.id == chat
        assert details["candidate_count"] == 1

        selected, details = select_auto_model(
            db, owner, [parse], "hello", "chat", [], False, ["hello"], 10, "only-special-purpose",
        )
        assert selected is None
        assert "通常チャット" in details["reason"]


def test_auto_retry_count_defaults_to_three_for_legacy_settings(signed):
    with SessionLocal() as db:
        settings = db.get(Settings, "settings")
        settings.data = {"setup_complete": False, "allowed_domains": []}
        db.commit()
    assert signed.get("/api/v1/settings").json()["data"]["auto_retry_count"] == 3


def test_auto_retry_count_repairs_malformed_legacy_snapshot_value(signed, monkeypatch):
    model_id = make_model({}, model_id="legacy-retry-count")
    with SessionLocal() as db:
        settings = db.get(Settings, "settings")
        settings.data = {"default_model_id": "auto", "auto_model_ids": [model_id],
                         "auto_retry_count": None, "allowed_domains": []}
        db.commit()
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    response = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat"},
        headers={"Idempotency-Key": "malformed-retry-count"})
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        run = db.get(Run, response.json()["run_id"])
        assert run.data["snapshot"]["auto_retry_count"] == 3
    assert engine.auto_retry_count(None) == 3
    assert engine.auto_retry_count("3") == 3
    assert engine.auto_retry_count(True) == 3
    assert engine.auto_retry_count(0) == 0


def test_provider_rate_limit_ignores_malformed_boolean_value():
    async def verify():
        # A boolean is not a valid request count, even though bool subclasses int.
        await engine.wait_for_provider_slot({"id": "boolean-rate-limit", "rate_limit_rpm": True})
        assert not engine._PROVIDER_REQUESTS["boolean-rate-limit"]

    asyncio.run(verify())


def test_output_tokens_rejects_boolean_provider_usage():
    assert engine.output_tokens({"output_tokens": True}) is None
    assert engine.output_tokens({"completion_tokens": False}) is None
    assert engine.output_tokens({"candidates_token_count": 12}) == 12


def test_auto_reliability_is_scope_aware_and_success_clears_consecutive_cooldown(signed):
    model_id = make_model({}, model_id="reliability")
    with SessionLocal() as db:
        model = db.get(Model, model_id)
        owner = db.scalar(select(User.id))
        base = engine.now()
        # Thinking failures must not poison an otherwise healthy Chat route.
        for offset in (0, 1):
            record_reliability(db, owner, model.id, model.data["provider_id"], "thinking", "failure",
                               "timeout", current=base + timedelta(minutes=offset))
        db.commit()
        chat = reliability(db, owner, model.id, model.data["provider_id"], "chat", base + timedelta(minutes=2))
        thinking = reliability(db, owner, model.id, model.data["provider_id"], "thinking", base + timedelta(minutes=2))
        assert chat["model_cooldown_until"] is None
        assert thinking["model_cooldown_until"] is not None
        record_reliability(db, owner, model.id, model.data["provider_id"], "thinking", "success",
                           current=base + timedelta(minutes=3))
        db.commit()
        recovered = reliability(db, owner, model.id, model.data["provider_id"], "thinking", base + timedelta(minutes=3))
        assert recovered["model_cooldown_until"] is None


def test_auto_reliability_classifies_retryable_and_non_availability_errors(signed):
    response = httpx.Response(429, headers={"retry-after": "30"}, request=httpx.Request("POST", "https://provider.test"))
    assert classify_failure(httpx.HTTPStatusError("limited", request=response.request, response=response)) == "rate_limit"
    assert classify_failure(ProviderContextLimitError("context limit")) == "context"
    assert classify_failure(ValueError("tool calling is not supported")) == "tool"
    unauthorized = httpx.Response(401, request=response.request)
    assert classify_failure(httpx.HTTPStatusError("unauthorized", request=unauthorized.request, response=unauthorized)) == "auth"


def test_auto_speed_uses_decayed_scope_specific_timings_and_normalizes_output(signed):
    fast = make_model({}, model_id="fast")
    slow = make_model({}, model_id="slow")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        fast_model, slow_model = db.get(Model, fast), db.get(Model, slow)
        base = engine.now()
        for offset in range(4):
            current = base + timedelta(minutes=offset)
            record_reliability(
                db, owner, fast, fast_model.data["provider_id"], "chat", "success", current=current,
                first_output_ms=100, completion_ms=1_000, output_tokens=100,
            )
            record_reliability(
                db, owner, slow, slow_model.data["provider_id"], "chat", "success", current=current,
                first_output_ms=1_000, completion_ms=10_000, output_tokens=100,
            )
        # Thinking evidence must not affect the chat route.
        record_reliability(
            db, owner, slow, slow_model.data["provider_id"], "thinking", "success", current=base,
            first_output_ms=10, completion_ms=100, output_tokens=100,
        )
        db.commit()
        selected, details = select_auto_model(
            db, owner, [fast, slow], "hello", "chat", [], False, ["hello"], 10, "speed-preference",
        )
        fast_speed = speed(db, owner, fast, fast_model.data["provider_id"], "chat", base + timedelta(minutes=4))
        assert selected.id == fast
        assert details["speed"]["score"] > 0
        assert "応答速度の実績" in details["reason"]
        assert fast_speed["completion_normalized"] is True
        assert fast_speed["completion_value"] == 10


def test_auto_speed_falls_back_without_usage_and_ignores_failures(signed):
    model_id = make_model({}, model_id="speed-fallback")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        model = db.get(Model, model_id)
        record_reliability(
            db, owner, model_id, model.data["provider_id"], "chat", "failure", "timeout",
            first_output_ms=1, completion_ms=1, output_tokens=1,
        )
        record_reliability(
            db, owner, model_id, model.data["provider_id"], "chat", "success",
            first_output_ms=200, completion_ms=2_000,
        )
        db.commit()
        details = speed(db, owner, model_id, model.data["provider_id"], "chat")
        assert round(details["first_output_ms"]) == 200
        assert round(details["completion_value"]) == 2_000
        assert details["completion_normalized"] is False


def test_auto_speed_needs_enough_comparable_evidence(signed):
    first = make_model({}, model_id="one-sample-fast")
    second = make_model({}, model_id="one-sample-slow")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        first_model, second_model = db.get(Model, first), db.get(Model, second)
        profile = routing_profile("hello", "chat", [], False, 13)
        record_reliability(db, owner, first, first_model.data["provider_id"], "chat", "success",
                           first_output_ms=10, completion_ms=10, output_tokens=1, profile=profile)
        record_reliability(db, owner, second, second_model.data["provider_id"], "chat", "success",
                           first_output_ms=10_000, completion_ms=10_000, output_tokens=1, profile=profile)
        db.commit()
        _, details = select_auto_model(db, owner, [first, second], "hello", "chat", [], False,
                                       ["hello"], 10, "one-sample-speed")
        assert details["speed"]["score"] == 0


def test_auto_avoids_unknown_context_after_matching_context_failure(signed):
    model_id = make_model({}, context=None, model_id="unknown-context-failure")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        model = db.get(Model, model_id)
        record_reliability(db, owner, model_id, model.data["provider_id"], "chat", "failure", "context",
                           required_tokens=100)
        db.commit()
        selected, details = select_auto_model(db, owner, [model_id], "x" * 200, "chat", [], False,
                                              ["x" * 200], 10, "known-context-failure")
        assert selected is None
        assert "Context Window" in details["reason"]


def test_auto_retries_only_before_visible_output(signed, monkeypatch):
    first = make_model({}, context=None, model_id="partial-first")
    second = make_model({}, context=None, model_id="partial-second")
    assert signed.put("/api/v1/settings", json={"default_model_id": "auto", "auto_model_ids": [first, second],
                                                  "allowed_domains": []}).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat"}, headers={"Idempotency-Key": "partial-output"})
    attempts = []

    class FakeAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            attempts.append(model)
            yield {"kind": "text", "text": "partial"}
            response = httpx.Response(503, request=httpx.Request("POST", "https://provider.test"))
            raise httpx.HTTPStatusError("temporary", request=response.request, response=response)

    monkeypatch.setattr(engine, "Adapter", FakeAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        assert db.get(Run, queued.json()["run_id"]).status == "failed"
    assert len(attempts) == 1


def test_auto_success_records_first_output_and_completion_timings(signed, monkeypatch):
    model_id = make_model({}, model_id="timed")
    assert signed.put("/api/v1/settings", json={"default_model_id": "auto", "auto_model_ids": [model_id],
                                                  "allowed_domains": []}).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat"}, headers={"Idempotency-Key": "timed-auto"})

    class FakeAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            yield {"kind": "text", "text": "hello"}
            yield {"kind": "response", "message": {"role": "assistant", "content": "hello"},
                   "tool_calls": [], "usage": {"output_tokens": 5}}

    monkeypatch.setattr(engine, "Adapter", FakeAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        event = db.scalar(select(AutoReliabilityEvent).where(
            AutoReliabilityEvent.data["model_id"].as_string() == model_id,
            AutoReliabilityEvent.data["outcome"].as_string() == "success",
        ))
        assert event.data["first_output_ms"] >= 1
        assert event.data["completion_ms"] >= event.data["first_output_ms"]
        assert event.data["output_tokens"] == 5


def test_completed_answers_record_tps_for_manual_models_and_statistics(signed, monkeypatch):
    model_id = make_model({}, model_id="manual-timed")
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": model_id, "content": "hello", "mode": "chat"},
        headers={"Idempotency-Key": "manual-timed"})

    class FakeAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            yield {"kind": "text", "text": "hello"}
            yield {"kind": "response", "message": {"role": "assistant", "content": "hello"},
                   "tool_calls": [], "usage": {"output_tokens": 5}}

    monkeypatch.setattr(engine, "Adapter", FakeAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        event = db.scalar(select(PerformanceEvent).where(
            PerformanceEvent.data["model_id"].as_string() == model_id,
        ))
        assert event.data["output_tokens"] == 5
        assert event.data["generation_ms"] >= 1
        assert event.data["tokens_per_second"] > 0
        message = db.scalar(select(Message).where(Message.conversation_id == conversation,
                                                   Message.data["role"].as_string() == "assistant"))
        assert message.data["performance"] == {
            "output_tokens": 5,
            "generation_ms": event.data["generation_ms"],
            "tokens_per_second": event.data["tokens_per_second"],
        }
        assert not db.scalar(select(AutoReliabilityEvent).where(
            AutoReliabilityEvent.data["model_id"].as_string() == model_id,
        ))
    statistics = signed.get("/api/v1/settings/statistics")
    assert statistics.status_code == 200
    group = next(item for item in statistics.json()["groups"] if item["model_id"] == model_id)
    assert group["tps_count"] == 1
    assert group["tokens_per_second"] > 0


def test_completed_answer_without_usage_does_not_record_tps(signed, monkeypatch):
    model_id = make_model({}, model_id="manual-no-usage")
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": model_id, "content": "hello", "mode": "chat"},
        headers={"Idempotency-Key": "manual-no-usage"})

    class FakeAdapter:
        def __init__(self, provider, key):
            pass

        async def stream(self, model, history, tools, mode, settings):
            yield {"kind": "text", "text": "hello"}
            yield {"kind": "response", "message": {"role": "assistant", "content": "hello"},
                   "tool_calls": [], "usage": {}}

    monkeypatch.setattr(engine, "Adapter", FakeAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))
    with SessionLocal() as db:
        assert not db.scalar(select(PerformanceEvent).where(
            PerformanceEvent.data["model_id"].as_string() == model_id,
        ))
        message = db.scalar(select(Message).where(Message.conversation_id == conversation,
                                                   Message.data["role"].as_string() == "assistant"))
        assert "performance" not in message.data


def test_auto_retry_prefers_another_provider_and_honors_retry_after(signed):
    first = make_model({}, model_id="first-provider")
    second = make_model({}, model_id="second-provider")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        first_model, second_model = db.get(Model, first), db.get(Model, second)
        assert first_model.data["provider_id"] != second_model.data["provider_id"]
        selected, _ = select_auto_model(
            db, owner, [first, second], "hello", "chat", [], False, ["hello"], 10, "cross-provider",
            excluded_model_ids=(), prefer_other_provider_than=first_model.data["provider_id"],
        )
        assert selected.id == second
    base = engine.now()
    response = httpx.Response(429, headers={"retry-after": "90"}, request=httpx.Request("POST", "https://provider.test"))
    until = retry_after(httpx.HTTPStatusError("limited", request=response.request, response=response), base)
    assert until == base + timedelta(seconds=90)


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


def test_nvidia_nim_chat_incompatible_is_limited_to_known_models_and_400():
    response = httpx.Response(400, request=httpx.Request("POST", "https://integrate.api.nvidia.com/v1/chat/completions"))
    incompatible = httpx.HTTPStatusError("bad request", request=response.request, response=response)
    provider = {"preset_id": "nvidia-nim"}
    assert is_nvidia_nim_chat_incompatible(provider, "nvidia/nemotron-parse", incompatible)
    assert not is_nvidia_nim_chat_incompatible(provider, "nvidia/llama-3.3-nemotron-super-49b-v1.5", incompatible)
    assert not is_nvidia_nim_chat_incompatible({"preset_id": "openai"}, "nvidia/nemotron-parse", incompatible)


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
