import pytest
import httpx
from mix_agent.providers.adapters import Adapter
from mix_agent.providers.catalog import PRESETS
from mix_agent.db.models import Model, Provider, Settings, User
from mix_agent.db.session import SessionLocal
from sqlalchemy import select

@pytest.mark.parametrize("kind", ["openai", "anthropic", "gemini", "openrouter", "ollama", "lmstudio", "compatible"])
async def test_model_discovery_conservative_capabilities(kind, monkeypatch):
    original = httpx.AsyncClient
    def handle(request):
        if kind == "anthropic":
            assert request.headers["x-api-key"] == "test-key"
        if kind == "gemini":
            assert request.headers["x-goog-api-key"] == "test-key"
            return httpx.Response(200, json={"models":[{"name":"models/example", "inputTokenLimit":12345}]})
        return httpx.Response(200, json={"data":[{"id":"example"}]})
    monkeypatch.setattr(httpx,"AsyncClient",lambda **kw: original(transport=httpx.MockTransport(handle), **kw))
    models = await Adapter({"kind":kind,"base_url":"https://provider.example/v1"},"test-key").list_models()
    assert models[0]["model_id"] == "example"
    assert all(v is None for v in models[0]["capabilities"].values())

def test_override_wins_over_detection():
    adapter = Adapter({"kind":"compatible","base_url":"https://example.com"},"")
    assert adapter.get_capabilities({"capabilities":{"tools":True},"overrides":{"tools":False}})["tools"] is False


def test_catalog_has_fifty_unique_presets_and_custom_is_last():
    assert len(PRESETS) == 50
    assert len({item["id"] for item in PRESETS}) == 50
    assert PRESETS[-1]["id"] == "custom"


def test_catalog_endpoint_and_custom_provider_validation(signed, monkeypatch):
    async def no_models(self):
        return []

    monkeypatch.setattr(Adapter, "list_models", no_models)
    catalog = signed.get("/api/v1/provider-presets")
    assert catalog.status_code == 200
    assert len(catalog.json()) == 50
    created = signed.post("/api/v1/providers", json={
        "name": "Fast Groq", "preset_id": "groq", "base_url": "", "api_key": "test-key", "allow_private": True,
    })
    assert created.status_code == 200, created.text
    assert created.json()["data"]["kind"] == "compatible"
    assert created.json()["data"]["base_url"] == "https://api.groq.com/openai/v1"
    missing_url = signed.post("/api/v1/providers", json={
        "name": "Custom", "preset_id": "custom", "kind": "compatible", "base_url": "",
    })
    assert missing_url.status_code == 422
    invalid_kind = signed.post("/api/v1/providers", json={
        "name": "Bad", "preset_id": "custom", "kind": "openrouter", "base_url": "https://example.com/v1",
    })
    assert invalid_kind.status_code == 422
    invalid_preset = signed.post("/api/v1/providers", json={
        "name": "Bad", "preset_id": "missing", "base_url": "https://example.com/v1",
    })
    assert invalid_preset.status_code == 422


async def test_known_context_is_filled_when_provider_omits_it(monkeypatch):
    original = httpx.AsyncClient

    def handle(request):
        return httpx.Response(200, json={"data": [{"id": "gpt-4o-mini"}, {"id": "unknown-model"}]})

    monkeypatch.setattr(httpx, "AsyncClient", lambda **kw: original(transport=httpx.MockTransport(handle), **kw))
    models = await Adapter({"kind": "openai", "base_url": "https://provider.example/v1"}, "test-key").list_models()
    assert models[0]["context_window"] == 128000
    assert models[0]["context_source"] == "catalog"
    assert models[1]["context_window"] is None


def test_provider_save_syncs_models_and_auto_candidates(signed, monkeypatch):
    async def list_models(self):
        return [
            {"model_id": "gpt-4o-mini", "name": "GPT", "capabilities": {}, "context_window": 128000,
             "context_source": "catalog", "source": "provider_api", "overrides": {}},
            {"model_id": "unknown", "name": "Unknown", "capabilities": {}, "context_window": None,
             "context_source": None, "source": "provider_api", "overrides": {}},
        ]

    monkeypatch.setattr(Adapter, "list_models", list_models)
    response = signed.post("/api/v1/providers", json={
        "name": "OpenAI", "preset_id": "openai", "base_url": "", "api_key": "test-key", "allow_private": True,
    })
    assert response.status_code == 200, response.text
    assert response.json()["model_sync"] == {"status": "ok", "count": 2, "auto_count": 1, "error": None}
    with SessionLocal() as db:
        settings = db.get(Settings, "settings")
        models = list(db.scalars(select(Model)))
        assert settings.data["default_model_id"] == "auto"
        assert len(settings.data["auto_model_ids"]) == 1
        assert {model.data["model_id"] for model in models} == {"gpt-4o-mini", "unknown"}


def test_provider_sync_failure_keeps_saved_provider(signed, monkeypatch):
    async def fail(self):
        raise RuntimeError("offline")

    monkeypatch.setattr(Adapter, "list_models", fail)
    response = signed.post("/api/v1/providers", json={
        "name": "OpenAI", "preset_id": "openai", "base_url": "", "api_key": "test-key", "allow_private": True,
    })
    assert response.status_code == 200, response.text
    assert response.json()["model_sync"]["status"] == "failed"
    assert len(signed.get("/api/v1/providers").json()) == 1


def test_auto_model_setting_accepts_more_than_one_hundred_ids(signed):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "openai"})
        db.add(provider)
        db.flush()
        models = []
        for index in range(101):
            model = Model(owner_id=owner, data={"provider_id": provider.id, "model_id": f"model-{index}"})
            db.add(model)
            models.append(model)
        db.commit()
        ids = [model.id for model in models]
    response = signed.put("/api/v1/settings", json={"auto_model_ids": ids, "allowed_domains": []})
    assert response.status_code == 200, response.text
    assert len(response.json()["data"]["auto_model_ids"]) == 101
