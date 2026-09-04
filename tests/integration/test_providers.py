import pytest
import httpx
from mix_agent.api import routes
from mix_agent.providers.adapters import Adapter
from mix_agent.providers.catalog import PRESETS, get_preset
from mix_agent.providers.metadata import resolve
from mix_agent.db.models import Model, Provider, Settings, User
from mix_agent.db.session import SessionLocal
from sqlalchemy import select

@pytest.mark.parametrize("kind", ["openai", "anthropic", "gemini", "openrouter", "ollama", "lmstudio", "compatible"])
async def test_model_discovery_conservative_capabilities(kind, monkeypatch):
    original = httpx.AsyncClient
    def handle(request):
        if kind == "ollama":
            if request.url.path == "/api/tags":
                return httpx.Response(200, json={"models":[{"name":"example"}]})
            if request.url.path == "/api/show":
                return httpx.Response(200, json={"model_info": {}})
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
    assert models[0]["context_source"] == "official_catalog"
    assert models[1]["context_window"] is None


async def test_anthropic_discovery_does_not_duplicate_v1_path(monkeypatch):
    original = httpx.AsyncClient

    def handle(request):
        assert request.url.path == "/v1/models"
        return httpx.Response(200, json={"data": [{"id": "claude-test"}]})

    monkeypatch.setattr(httpx, "AsyncClient", lambda **kw: original(transport=httpx.MockTransport(handle), **kw))
    models = await Adapter(
        {"kind": "anthropic", "base_url": "https://provider.example/v1"}, "test-key"
    ).list_models()
    assert models[0]["model_id"] == "claude-test"


async def test_gemini_discovery_does_not_duplicate_v1beta_path(monkeypatch):
    original = httpx.AsyncClient

    def handle(request):
        assert request.url.path == "/v1beta/models"
        return httpx.Response(200, json={"models": [{"name": "models/gemini-test"}]})

    monkeypatch.setattr(httpx, "AsyncClient", lambda **kw: original(transport=httpx.MockTransport(handle), **kw))
    models = await Adapter(
        {"kind": "gemini", "base_url": "https://provider.example/v1beta"}, "test-key"
    ).list_models()
    assert models[0]["model_id"] == "gemini-test"


async def test_context_limit_normalizes_compatible_and_ollama_responses(monkeypatch):
    original = httpx.AsyncClient

    def compatible(request):
        return httpx.Response(200, json={"data": [{"id": "compatible", "max_context_tokens": "32768"}]})

    monkeypatch.setattr(httpx, "AsyncClient", lambda **kw: original(transport=httpx.MockTransport(compatible), **kw))
    models = await Adapter({"kind": "compatible", "base_url": "https://provider.example/v1"}, "test-key").list_models()
    assert models[0]["context_window"] == 32768
    assert models[0]["context_source"] == "provider_api"

    def ollama(request):
        if request.url.path == "/api/tags":
            return httpx.Response(200, json={"models": [{"name": "llama3"}]})
        assert request.url.path == "/api/show"
        return httpx.Response(200, json={"model_info": {"llama.context_length": 8192}})

    monkeypatch.setattr(httpx, "AsyncClient", lambda **kw: original(transport=httpx.MockTransport(ollama), **kw))
    models = await Adapter({"kind": "ollama", "base_url": "http://ollama.example:11434/v1"}, "").list_models()
    assert models[0]["context_window"] == 8192
    assert models[0]["context_source"] == "provider_api"


def test_context_limit_rejects_boolean_provider_values():
    from mix_agent.providers.metadata import context_limit

    assert context_limit({"context_length": True}) is None
    assert context_limit({"limits": {"context": False}}) is None
    assert context_limit({"context_length": 8192}) == 8192


def test_each_preset_declares_transport_discovery_and_metadata_chain():
    assert len(PRESETS) == 50
    for preset in PRESETS:
        assert preset["transport_id"]
        assert preset["discovery_id"]
        assert preset["metadata_resolver_ids"]
    assert get_preset("groq")["transport_id"] == "openai_compatible"
    assert get_preset("anthropic")["transport_id"] == "anthropic_messages"
    assert get_preset("ollama")["transport_id"] == "ollama"


async def test_metadata_resolver_prefers_provider_api_and_records_evidence(monkeypatch):
    async def empty_models_dev(kind, model_id):
        return {}

    monkeypatch.setattr("mix_agent.providers.metadata.models_dev", empty_models_dev)
    result = await resolve("openai", "gpt-4o-mini", {"id": "gpt-4o-mini", "context_length": 64_000})
    evidence = result["metadata"]["context_window"]
    assert evidence["value"] == 64_000
    assert evidence["source"] == "provider_api"
    assert evidence["confidence"] == "official"
    assert result["metadata_candidates"]["official_catalog"]["context_window"]["value"] == 128_000

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


def test_provider_sync_excludes_known_special_purpose_models_from_auto(signed, monkeypatch):
    async def list_models(self):
        return [
            {"model_id": "nvidia/nemotron-parse", "name": "Parse", "capabilities": {"chat": False},
             "context_window": 8192, "context_source": "provider_api", "source": "provider_api", "overrides": {}},
            {"model_id": "nvidia/llama-3.3-nemotron-super-49b-v1.5", "name": "Chat", "capabilities": {},
             "context_window": 8192, "context_source": "provider_api", "source": "provider_api", "overrides": {}},
        ]

    monkeypatch.setattr(Adapter, "list_models", list_models)
    response = signed.post("/api/v1/providers", json={
        "name": "NVIDIA NIM", "preset_id": "nvidia-nim", "base_url": "", "api_key": "test-key", "allow_private": True,
    })
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        settings = db.get(Settings, "settings")
        models = {model.data["model_id"]: model for model in db.scalars(select(Model))}
        assert models["nvidia/nemotron-parse"].id not in settings.data["auto_model_ids"]
        assert models["nvidia/llama-3.3-nemotron-super-49b-v1.5"].id in settings.data["auto_model_ids"]

        settings.data = {**settings.data, "auto_model_ids": [models["nvidia/nemotron-parse"].id]}
        db.commit()
        provider = db.scalar(select(Provider))
        import asyncio
        asyncio.run(routes.sync_provider_models(db, db.scalar(select(User.id)), provider))
        assert models["nvidia/nemotron-parse"].id not in settings.data["auto_model_ids"]


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


@pytest.mark.asyncio
async def test_provider_sync_preserves_manual_context_override(signed, monkeypatch):
    async def list_models(self):
        return [{"model_id": "custom", "name": "Custom", "capabilities": {}, "context_window": 8192,
                 "context_source": "provider_api", "source": "provider_api", "overrides": {}}]

    monkeypatch.setattr(Adapter, "list_models", list_models)
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "compatible"})
        db.add(provider)
        db.flush()
        model = Model(owner_id=owner, data={"provider_id": provider.id, "model_id": "custom",
                                            "context_window": 16384, "context_window_override": 16384,
                                            "context_source": "manual"})
        db.add(model)
        db.flush()
        model_id = model.id
        await routes.sync_provider_models(db, owner, provider)
        db.commit()
        refreshed = db.get(Model, model.id)
        assert refreshed.data["context_window"] == 16384
        assert refreshed.data["context_window_override"] == 16384
        assert refreshed.data["context_source"] == "manual"
        assert refreshed.data["provider_context_window"] == 8192

    cleared = signed.patch(f"/api/v1/models/{model_id}", json={"context_window_override": None})
    assert cleared.status_code == 200, cleared.text
    assert cleared.json()["data"]["context_window"] == 8192
    assert cleared.json()["data"]["context_source"] == "provider_api"


@pytest.mark.asyncio
async def test_provider_sync_updates_existing_model_without_creating_a_duplicate(signed, monkeypatch):
    async def list_models(self):
        return [{"model_id": "custom", "name": "Custom", "capabilities": {}, "context_window": 16384,
                 "context_source": "provider_api", "source": "provider_api", "overrides": {}}]

    monkeypatch.setattr(Adapter, "list_models", list_models)
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "compatible"})
        db.add(provider)
        db.flush()
        model = Model(owner_id=owner, data={"provider_id": provider.id, "model_id": "custom",
                                            "context_window": 8192, "context_source": "provider_api"})
        db.add(model)
        db.flush()
        original_id = model.id

        result = await routes.sync_provider_models(db, owner, provider)
        db.commit()

        models = [row for row in db.scalars(select(Model)) if row.data["provider_id"] == provider.id]
        assert result["count"] == 1
        assert len(models) == 1
        assert models[0].id == original_id
        assert models[0].data["context_window"] == 16384
        assert models[0].data["provider_context_window"] == 16384
