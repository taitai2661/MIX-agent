import asyncio

from mix_agent.api import routes
from mix_agent.db.models import Model, ModelUsageEvent, Provider, User
from mix_agent.db.session import SessionLocal
from mix_agent.runs import engine
from sqlalchemy import select


def make_model(pricing=None, model_id="priced"):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "openai"})
        db.add(provider)
        db.flush()
        data = {"provider_id": provider.id, "model_id": model_id, "capabilities": {}, "context_window": 20000}
        if pricing is not None:
            data["pricing_override"] = pricing
        model = Model(owner_id=owner, data=data)
        db.add(model)
        db.commit()
        return model.id


def _drive(signed, monkeypatch, model_id, usage, key):
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    queued = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": model_id, "content": "hello", "mode": "chat"}, headers={"Idempotency-Key": key})

    class FakeAdapter:
        def __init__(self, provider, secret):
            pass

        async def stream(self, model, history, tools, mode, settings):
            yield {"kind": "text", "text": "hello"}
            yield {"kind": "response", "message": {"role": "assistant", "content": "hello"},
                   "tool_calls": [], "usage": usage}

    monkeypatch.setattr(engine, "Adapter", FakeAdapter)
    asyncio.run(engine.drive(queued.json()["run_id"]))


def test_usage_recorded_and_reported_with_cost(signed, monkeypatch):
    model_id = make_model({"input": 3, "output": 15, "cache_read": 0.3}, model_id="priced-cost")
    _drive(signed, monkeypatch, model_id, {"input_tokens": 1000, "output_tokens": 2000}, "usage-cost")
    with SessionLocal() as db:
        event = db.scalar(select(ModelUsageEvent).where(
            ModelUsageEvent.data["model_id"].as_string() == "priced-cost"))
        assert event.data["input_tokens"] == 1000
        assert event.data["output_tokens"] == 2000
        assert event.data["input_source"] == "provider"
        assert event.data["pricing_source"] == "manual"
        assert event.data["cost_usd"] == 0.033
    usage = signed.get("/api/v1/settings/usage?days=30")
    assert usage.status_code == 200, usage.text
    body = usage.json()
    assert body["total"]["requests"] == 1
    assert body["total"]["input_tokens"] == 1000
    assert body["total"]["output_tokens"] == 2000
    assert body["total"]["cost_usd"] == 0.033
    group = next(item for item in body["groups"] if item["model_id"] == "priced-cost")
    assert group["pricing_source"] == "manual"
    assert group["unpriced"] is False


def test_unpriced_usage_is_reported_separately(signed, monkeypatch):
    model_id = make_model(model_id="priced-unknown")
    _drive(signed, monkeypatch, model_id, {"input_tokens": 10, "output_tokens": 5}, "usage-unknown")
    usage = signed.get("/api/v1/settings/usage?days=30")
    assert usage.status_code == 200, usage.text
    body = usage.json()
    assert body["total"]["cost_usd"] is None
    assert body["total"]["unpriced_requests"] == 1
    group = next(item for item in body["groups"] if item["model_id"] == "priced-unknown")
    assert group["unpriced"] is True
    assert group["cost_usd"] is None


def test_usage_reports_provider_breakdown_with_names(signed, monkeypatch):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "compatible", "name": "NVIDIA NIM"})
        db.add(provider)
        db.flush()
        model = Model(owner_id=owner, data={
            "provider_id": provider.id, "model_id": "nvidia/nemotron-super",
            "capabilities": {}, "context_window": 131_072,
            "pricing_override": {"input": 1, "output": 2},
        })
        db.add(model)
        db.commit()
        model_key, provider_key = model.id, provider.id
    _drive(signed, monkeypatch, model_key, {"input_tokens": 100, "output_tokens": 50}, "usage-provider")
    usage = signed.get("/api/v1/settings/usage?days=30")
    assert usage.status_code == 200, usage.text
    body = usage.json()
    row = next((item for item in body["providers"] if item["provider_id"] == provider_key), None)
    assert row is not None, body.get("providers")
    assert row["provider_name"] == "NVIDIA NIM"
    assert row["requests"] == 1
    assert row["input_tokens"] == 100
    assert row["output_tokens"] == 50
    assert row["total_tokens"] == 150
    assert row["unpriced"] is False
    # 100 input * $1/M + 50 output * $2/M = $0.0002
    assert abs(row["cost_usd"] - 0.0002) < 1e-9
    group = next(item for item in body["groups"] if item["model_id"] == "nvidia/nemotron-super")
    assert group["provider_name"] == "NVIDIA NIM"
    assert group["mode"] == "chat"
