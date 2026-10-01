import httpx
import pytest
from mix_agent.api import routes
from mix_agent.db.models import Model, Provider, Settings, User
from mix_agent.db.session import SessionLocal
from mix_agent.providers.adapters import Adapter
from mix_agent.providers.catalog import PRESETS, get_preset
from mix_agent.providers.metadata import resolve
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


def test_catalog_has_fifty_two_unique_presets_and_custom_is_last():
    assert len(PRESETS) == 52
    assert len({item["id"] for item in PRESETS}) == 52
    assert PRESETS[-1]["id"] == "custom"


def test_catalog_endpoint_and_custom_provider_validation(signed, monkeypatch):
    async def no_models(self):
        return []

    monkeypatch.setattr(Adapter, "list_models", no_models)
    catalog = signed.get("/api/v1/provider-presets")
    assert catalog.status_code == 200
    assert len(catalog.json()) == 52
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
    from mix_agent.providers import metadata

    async def fake_lookup(provider_ids, model_id):
        return {"context_window": 128_000} if model_id == "gpt-4o-mini" else {}

    monkeypatch.setattr(metadata.model_info, "lookup", fake_lookup)
    original = httpx.AsyncClient

    def handle(request):
        return httpx.Response(200, json={"data": [{"id": "gpt-4o-mini"}, {"id": "unknown-model"}]})

    monkeypatch.setattr(httpx, "AsyncClient", lambda **kw: original(transport=httpx.MockTransport(handle), **kw))
    models = await Adapter({"kind": "openai", "base_url": "https://provider.example/v1"}, "test-key").list_models()
    assert models[0]["context_window"] == 128000
    assert models[0]["context_source"] == "model_info"
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
    assert len(PRESETS) == 52
    for preset in PRESETS:
        assert preset["transport_id"]
        assert preset["discovery_id"]
        assert preset["metadata_resolver_ids"]
    assert get_preset("groq")["transport_id"] == "openai_compatible"
    assert get_preset("anthropic")["transport_id"] == "anthropic_messages"
    assert get_preset("ollama")["transport_id"] == "ollama"


async def test_metadata_resolver_prefers_provider_api_and_records_evidence(monkeypatch):
    from mix_agent.providers import metadata

    async def fake_lookup(provider_ids, model_id):
        return {"context_window": 128_000, "max_output_tokens": 16_384}

    monkeypatch.setattr(metadata.model_info, "lookup", fake_lookup)
    result = await resolve("openai", "gpt-4o-mini", {"id": "gpt-4o-mini", "context_length": 64_000})
    evidence = result["metadata"]["context_window"]
    assert evidence["value"] == 64_000
    assert evidence["source"] == "provider_api"
    assert evidence["confidence"] == "official"
    assert result["metadata_candidates"]["model_info"]["context_window"]["value"] == 128_000
    assert result["metadata"]["max_output_tokens"]["value"] == 16_384


async def test_metadata_resolver_reads_model_info_when_provider_says_nothing(monkeypatch):
    from mix_agent.providers import metadata

    async def fake_lookup(provider_ids, model_id):
        seen["keys"] = tuple(provider_ids)
        return {"context_window": 131_072}

    seen = {}
    monkeypatch.setattr(metadata.model_info, "lookup", fake_lookup)
    result = await resolve("compatible", "nvidia/llama-3.3-nemotron-super-49b-v1.5", {},
                           preset_id="nvidia-nim")
    evidence = result["metadata"]["context_window"]
    assert evidence["value"] == 131_072
    assert evidence["source"] == "model_info"
    assert evidence["confidence"] == "external"
    assert seen["keys"] == ("nvidia",)
    assert result["metadata_candidates"]["model_info"]["context_window"]["value"] == 131_072


async def test_metadata_resolver_leaves_context_unknown_without_any_source(monkeypatch):
    from mix_agent.providers import metadata

    async def empty_lookup(provider_ids, model_id):
        return {}

    monkeypatch.setattr(metadata.model_info, "lookup", empty_lookup)
    result = await resolve("compatible", "some/undocumented-model", {}, preset_id="nvidia-nim")
    assert result["metadata"] == {}


def test_context_limit_accepts_nested_limit_shape():
    from mix_agent.providers.metadata import context_limit

    assert context_limit({"limit": {"context": 131_072, "output": 65_536}}) == 131_072
    assert context_limit({"limit": {"context": False}}) is None


async def test_documented_capabilities_fill_only_unknown_slots(monkeypatch):
    from mix_agent.providers import metadata

    async def fake_lookup(provider_ids, model_id):
        return {"capabilities": {"tools": True, "vision": False, "reasoning": None,
                                 "structured_output": True}}

    monkeypatch.setattr(metadata.model_info, "lookup", fake_lookup)
    original = httpx.AsyncClient

    def undocumented(request):
        # OpenAI-style listings carry no capability hints at all.
        return httpx.Response(200, json={"data": [{"id": "example"}]})

    monkeypatch.setattr(httpx, "AsyncClient",
                        lambda **kw: original(transport=httpx.MockTransport(undocumented), **kw))
    models = await Adapter({"kind": "openai", "base_url": "https://provider.example/v1"}, "key").list_models()
    assert models[0]["capabilities"] == {
        "chat": None, "tools": True, "vision": False, "reasoning": None, "structured_output": True,
    }

    def with_parameters(request):
        # The wire listing is authoritative wherever it does speak.
        return httpx.Response(200, json={"data": [{"id": "example", "supported_parameters": ["tools"]}]})

    monkeypatch.setattr(httpx, "AsyncClient",
                        lambda **kw: original(transport=httpx.MockTransport(with_parameters), **kw))
    models = await Adapter({"kind": "openai", "base_url": "https://provider.example/v1"}, "key").list_models()
    caps = models[0]["capabilities"]
    assert caps["tools"] is True
    # A parameter listing that omits a feature denies it; documentation never
    # overrides that, and it only fills the slots the listing left silent.
    assert caps["reasoning"] is False
    assert caps["structured_output"] is False
    assert caps["vision"] is False


def test_nim_non_chat_model_families_are_excluded():
    from mix_agent.providers.model_roles import chat_capability

    for model_id in ("nvidia/nv-embed-v1", "nvidia/rerank-qa-mistral-4b",
                     "black-forest-labs/flux.1-dev", "baai/bge-m3",
                     "nvidia/magpie-tts-zeroshot", "nvidia/nemotron-parse"):
        assert chat_capability("compatible", model_id) is False, model_id
    for model_id in ("nvidia/llama-3.3-nemotron-super-49b-v1.5",
                     "nvidia/nemotron-voicechat", "meta/llama-3.3-70b-instruct"):
        assert chat_capability("compatible", model_id) is None, model_id


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


def test_delete_provider_removes_models_and_clears_references(signed):
    from mix_agent.auth.security import store_secret
    from mix_agent.db.models import Agent, Conversation, Secret

    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        secret_id = store_secret(db, owner, "test-key", "provider")
        provider = Provider(owner_id=owner, data={"kind": "openai", "secret_id": secret_id})
        other = Provider(owner_id=owner, data={"kind": "openai"})
        db.add_all([provider, other])
        db.flush()
        model = Model(owner_id=owner, data={"provider_id": provider.id, "model_id": "gpt-4o-mini"})
        kept = Model(owner_id=owner, data={"provider_id": other.id, "model_id": "kept"})
        db.add_all([model, kept])
        db.flush()
        settings = db.get(Settings, "settings")
        settings.data = {**settings.data, "auto_model_ids": [model.id, kept.id], "default_model_id": model.id}
        agent = Agent(owner_id=owner, data={"name": "A", "model_id": model.id})
        conversation = Conversation(owner_id=owner, data={"title": "c", "selection": {"model_id": model.id}})
        db.add_all([agent, conversation])
        db.commit()
        provider_id, model_id, kept_id, agent_id, conversation_id = (
            provider.id, model.id, kept.id, agent.id, conversation.id
        )

    response = signed.delete(f"/api/v1/providers/{provider_id}")
    assert response.status_code == 200, response.text
    assert response.json() == {"ok": True}

    with SessionLocal() as db:
        assert db.get(Provider, provider_id) is None
        assert db.get(Model, model_id) is None
        assert db.get(Secret, secret_id) is None
        assert db.get(Model, kept_id) is not None
        settings = db.get(Settings, "settings")
        assert settings.data["auto_model_ids"] == [kept_id]
        assert settings.data["default_model_id"] == "auto"
        assert db.get(Agent, agent_id).data["model_id"] == ""
        assert db.get(Conversation, conversation_id).data["selection"]["model_id"] == "auto"


def test_delete_provider_rejects_unknown_key(signed):
    assert signed.delete("/api/v1/providers/missing").status_code == 404


async def test_opencode_presets_resolve_metadata_through_model_info(monkeypatch):
    from mix_agent.providers import metadata

    seen = {}

    async def fake_lookup(provider_ids, model_id):
        seen["keys"] = tuple(provider_ids)
        return {"context_window": 1_000_000}

    monkeypatch.setattr(metadata.model_info, "lookup", fake_lookup)
    go = await resolve("compatible", "glm-5.3-flash", {}, preset_id="opencode-go")
    assert go["metadata"]["context_window"]["value"] == 1_000_000
    assert seen["keys"] == ("opencode-go",)
    zen = await resolve("compatible", "glm-5.3-flash", {}, preset_id="opencode-zen")
    assert zen["metadata"]["context_window"]["value"] == 1_000_000
    assert seen["keys"] == ("opencode",)


def test_opencode_models_use_the_documented_gateway_endpoint():
    go = Adapter(
        {"kind": "compatible", "base_url": "https://opencode.ai/zen/go/v1", "preset_id": "opencode-go"}, "key"
    )
    assert go.transport_for("glm-5.3-flash") == "openai_compatible"
    assert go.transport_for("minimax-m3") == "anthropic_messages"
    assert go.transport_for("grok-4.6") == "openai_responses"
    assert go.transport_for("model-added-later") == "openai_compatible"
    zen = Adapter(
        {"kind": "compatible", "base_url": "https://opencode.ai/zen/v1", "preset_id": "opencode-zen"}, "key"
    )
    # The same id is served on different endpoints by each gateway.
    assert zen.transport_for("minimax-m3") == "openai_compatible"
    assert zen.transport_for("claude-sonnet-5") == "anthropic_messages"
    assert zen.transport_for("gpt-5.1") == "openai_responses"


async def test_stream_dispatches_each_model_on_its_own_transport(monkeypatch):
    adapter = Adapter(
        {"kind": "compatible", "base_url": "https://opencode.ai/zen/go/v1", "preset_id": "opencode-go"}, "key"
    )
    used = []

    def recorder(name):
        async def record(self, model, messages, tools, mode, settings):
            used.append((name, model))
            if False:
                yield None

        return record

    monkeypatch.setattr(Adapter, "_compatible", recorder("openai_compatible"))
    monkeypatch.setattr(Adapter, "_openai", recorder("openai_responses"))
    monkeypatch.setattr(Adapter, "_anthropic", recorder("anthropic_messages"))
    for model_id in ("glm-5.3-flash", "grok-4.6", "minimax-m3"):
        async for _ in adapter.stream(model_id, [], [], "chat", {}):
            pass
    assert used == [
        ("openai_compatible", "glm-5.3-flash"),
        ("openai_responses", "grok-4.6"),
        ("anthropic_messages", "minimax-m3"),
    ]


def test_anthropic_transport_does_not_duplicate_v1_path():
    from mix_agent.providers.adapters import anthropic_base_url

    assert anthropic_base_url("https://opencode.ai/zen/go/v1") == "https://opencode.ai/zen/go"
    assert anthropic_base_url("https://api.anthropic.com/v1") == "https://api.anthropic.com"
    assert anthropic_base_url("https://api.anthropic.com") == "https://api.anthropic.com"


def test_only_opencode_go_declares_a_session_header():
    assert get_preset("opencode-go")["session_header"] == "x-opencode-session"
    assert get_preset("opencode-zen")["session_header"] is None
    assert get_preset("groq")["session_header"] is None


def test_gateway_headers_carry_the_conversation_session():
    provider = {"kind": "compatible", "base_url": "https://opencode.ai/zen/go/v1", "preset_id": "opencode-go"}
    adapter = Adapter(provider, "key")
    assert adapter.gateway_headers("conversation-1") == {"x-opencode-session": "conversation-1"}
    # Without a conversation (probes, memory jobs) the value stays stable.
    fallback = adapter.gateway_headers()
    assert fallback == Adapter(provider, "key").gateway_headers()
    assert set(fallback) == {"x-opencode-session"}
    groq = Adapter({"kind": "compatible", "base_url": "https://api.groq.com/openai/v1", "preset_id": "groq"}, "key")
    assert groq.gateway_headers() == {}


async def test_session_header_reaches_the_openai_client(monkeypatch):
    adapter = Adapter(
        {"kind": "compatible", "base_url": "https://opencode.ai/zen/go/v1", "preset_id": "opencode-go"}, "key"
    )
    captured = {}

    class FakeOpenAI:
        def __init__(self, **kwargs):
            captured.update(kwargs)

        async def __aenter__(self):
            return self

        async def __aexit__(self, *exc):
            return False

    async def stop(self, client, args):
        raise RuntimeError("captured")

    monkeypatch.setattr("mix_agent.providers.adapters.AsyncOpenAI", FakeOpenAI)
    monkeypatch.setattr(Adapter, "_compatible_create", stop)
    settings = {"max_output_tokens": 32, "_session_id": "conversation-1"}
    with pytest.raises(RuntimeError, match="captured"):
        async for _ in adapter.stream("deepseek-v4-flash", [{"role": "user", "content": "hi"}], [], "chat", settings):
            pass
    assert captured["default_headers"] == {"x-opencode-session": "conversation-1"}
