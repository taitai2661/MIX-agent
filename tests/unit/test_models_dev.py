import json
import time

import httpx
import pytest
from mix_agent import config
from mix_agent.providers import models_dev


@pytest.fixture(autouse=True)
def clean_cache():
    models_dev._catalog_cache.clear()
    yield
    models_dev._catalog_cache.clear()


def row(model_id, **fields):
    model = {
        "id": model_id,
        "limit": {"context": 1_000, "output": 500},
        "modalities": {"input": ["text"], "output": ["text"]},
        "cost": {"input": 1, "output": 2},
        "tool_call": True,
        "reasoning": False,
        "structured_output": None,
    }
    model.update(fields)
    return model


def rows(*models):
    return {model["id"]: model for model in models}


def payload(provider_id, *models):
    return {provider_id: {"id": provider_id, "models": rows(*models)}}


def matched(models, wire_id):
    key = models_dev._match(models, wire_id)
    return None if key is None else models[key]


def normalize(model):
    return models_dev._normalize(model)


def test_match_prefers_the_exact_provider_id():
    models = rows(row("meta/llama-3.3-70b-instruct"), row("llama-3.3-70b"))
    assert matched(models, "meta/llama-3.3-70b-instruct")["id"] == "meta/llama-3.3-70b-instruct"


def test_match_strips_a_vendor_prefix():
    models = rows(row("deepseek-v4-pro"))
    assert matched(models, "deepseek-ai/deepseek-v4-pro")["id"] == "deepseek-v4-pro"


def test_match_ignores_a_snapshot_suffix():
    models = rows(row("deepseek-ai/deepseek-v4-pro-0813"))
    assert matched(models, "deepseek-ai/deepseek-v4-pro")["id"] == "deepseek-ai/deepseek-v4-pro-0813"


def test_match_ignores_letter_case():
    models = rows(row("deepseek-v4-pro"))
    assert matched(models, "DeepSeek-V4-Pro")["id"] == "deepseek-v4-pro"


def test_match_reports_unknown_ids_as_a_miss():
    assert matched(rows(row("gpt-5")), "gpt-4o") is None
    assert matched({}, "gpt-4o") is None


def test_provider_keys_only_ever_name_registered_ids():
    assert models_dev.provider_keys("nvidia-nim") == ("nvidia",)
    assert models_dev.provider_keys("gemini") == ("google",)
    assert models_dev.provider_keys("together") == ("togetherai",)
    assert models_dev.provider_keys("opencode-zen") == ("opencode",)
    # A provider models.dev does not catalogue is never guessed at.
    assert models_dev.provider_keys("ollama") == ()


def test_normalize_reads_limits_and_modalities():
    facts = normalize(row("gpt-4o"))
    assert facts["context_window"] == 1_000
    assert facts["max_output_tokens"] == 500
    assert facts["modalities"] == {"input": ["text"], "output": ["text"]}
    assert facts["pricing"] == {"input": 1, "output": 2}


def test_normalize_falls_back_to_the_input_limit():
    facts = normalize(row("gpt-4o", limit={"context": 0, "input": 128_000, "output": 0}))
    assert facts["context_window"] == 128_000
    assert facts["max_output_tokens"] is None


def test_normalize_maps_capabilities_onto_wire_keys():
    facts = normalize(row("gpt-4o", tool_call=True, reasoning=True, structured_output=False))
    assert facts["capabilities"] == {
        "tools": True,
        "vision": False,
        "reasoning": True,
        "structured_output": False,
    }


def test_normalize_infers_vision_from_image_input():
    with_images = normalize(row("gpt-4o", modalities={"input": ["text", "image"], "output": ["text"]}))
    assert with_images["capabilities"]["vision"] is True
    text_only = normalize(row("gpt-4o"))
    assert text_only["capabilities"]["vision"] is False


def test_normalize_leaves_undocumented_capabilities_unknown():
    facts = normalize(row("gpt-4o", tool_call=None, reasoning=None, structured_output=None, modalities=None))
    assert facts["capabilities"] is None


def test_normalize_leaves_missing_pricing_unknown():
    facts = normalize(row("gpt-4o", cost=None))
    assert facts["pricing"] is None


async def test_lookup_reads_a_local_catalog(tmp_path, monkeypatch):
    target = tmp_path / "v1"
    target.mkdir()
    (target / "api.json").write_text(
        json.dumps(
            payload(
                "openai",
                row("gpt-4o"),
                row("gpt-4.1", limit={"context": 1_047_576, "output": 32_768}),
            )
        )
    )
    monkeypatch.setattr(config, "MODELS_DEV_URL", str(target))

    facts = await models_dev.lookup(("openai",), "gpt-4o")
    assert facts["context_window"] == 1_000
    facts = await models_dev.lookup(("openai",), "gpt-4.1")
    assert facts["context_window"] == 1_047_576
    assert await models_dev.lookup(("openai",), "unknown-model") == {}


async def test_lookup_tries_each_registered_provider_in_order(tmp_path, monkeypatch):
    target = tmp_path / "api.json"
    target.write_text(
        json.dumps(
            {
                **payload("nvidia"),
                **payload("openrouter", row("deepseek/deepseek-v4-pro")),
            }
        )
    )
    monkeypatch.setattr(config, "MODELS_DEV_URL", str(target))

    facts = await models_dev.lookup(("nvidia", "openrouter"), "deepseek-ai/deepseek-v4-pro")
    assert facts["context_window"] == 1_000


async def test_lookup_ignores_providers_outside_the_catalog(tmp_path, monkeypatch):
    target = tmp_path / "api.json"
    target.write_text(json.dumps(payload("openai", row("gpt-4o"))))
    monkeypatch.setattr(config, "MODELS_DEV_URL", str(target))

    assert await models_dev.lookup(("ollama",), "gpt-4o") == {}


async def test_lookup_returns_nothing_when_the_catalog_is_unreachable(monkeypatch):
    monkeypatch.setattr(config, "MODELS_DEV_URL", "/path/that/does/not/exist")
    assert await models_dev.lookup(("openai",), "gpt-4o") == {}
    assert "catalog" not in models_dev._catalog_cache


async def test_a_failed_refresh_serves_the_stale_catalog(monkeypatch):
    models_dev._catalog_cache["catalog"] = {
        "value": {"openai": rows(row("gpt-4o"))},
        "fetched_at": 0.0,
    }

    def unreachable(**kwargs):
        raise httpx.ConnectError("offline")

    monkeypatch.setattr(httpx, "AsyncClient", unreachable)
    monkeypatch.setattr(config, "MODELS_DEV_URL", "https://example.test/api.json")
    assert (await models_dev.lookup(("openai",), "gpt-4o"))["context_window"] == 1_000


async def test_the_http_catalog_is_downloaded_once_per_ttl(monkeypatch):
    calls = []
    original = httpx.AsyncClient

    def handle(request):
        calls.append(str(request.url))
        return httpx.Response(200, json=payload("openai", row("gpt-4o")))

    monkeypatch.setattr(
        httpx, "AsyncClient", lambda **kw: original(transport=httpx.MockTransport(handle), **kw)
    )
    monkeypatch.setattr(config, "MODELS_DEV_URL", "https://example.test/api.json")

    first = await models_dev.lookup(("openai",), "gpt-4o")
    second = await models_dev.lookup(("openai",), "gpt-4o")
    assert first["context_window"] == second["context_window"] == 1_000
    assert calls == ["https://example.test/api.json"]
    assert models_dev._catalog_cache["catalog"]["fetched_at"] <= time.monotonic()
