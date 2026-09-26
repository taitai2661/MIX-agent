import json
import time

import httpx
import pytest
from mix_agent import config
from mix_agent.providers import model_info


@pytest.fixture(autouse=True)
def clean_cache():
    model_info._catalog_cache.clear()
    yield
    model_info._catalog_cache.clear()


def entry(provider_model_id, model_id=None, **fields):
    return {"provider_id": "openai", "model_id": provider_model_id,
            "model": {"model_id": model_id or provider_model_id,
                      "context": {"window": 1_000, "max_output_tokens": 500},
                      "modalities": {"input": ["text"], "output": ["text"]},
                      "pricing": {"currency": "USD", "unit": "1M_tokens", "input": 1, "output": 2},
                      **fields}}


def matched(entries, wire_id):
    pair = model_info._match(entries, wire_id)
    return None if pair is None else pair[0]


def normalize(row):
    return model_info._normalize(model_info._split(row))


def test_match_prefers_the_exact_provider_id():
    entries = [entry("meta/llama-3.3-70b-instruct", "llama-3.3-70b-instruct"),
               entry("llama-3.3-70b")]
    assert matched(entries, "meta/llama-3.3-70b-instruct") is entries[0]


def test_match_strips_a_vendor_prefix():
    entries = [entry("deepseek-v4-pro")]
    assert matched(entries, "deepseek-ai/deepseek-v4-pro") is entries[0]


def test_match_ignores_a_snapshot_suffix():
    entries = [entry("deepseek-ai/deepseek-v4-pro-0813")]
    assert matched(entries, "deepseek-ai/deepseek-v4-pro") is entries[0]


def test_match_ignores_letter_case():
    entries = [entry("deepseek-v4-pro")]
    assert matched(entries, "DeepSeek-V4-Pro") is entries[0]


def test_match_reports_unknown_ids_as_a_miss():
    assert matched([entry("gpt-5")], "gpt-4o") is None
    assert matched([], "gpt-4o") is None


def test_provider_files_bundling_model_and_relationship_are_read():
    row = {
        "model": {"model_id": "nemotron-3-ultra", "context": {"window": 262_144, "max_output_tokens": 182_520}},
        "relationship": {"provider_id": "nvidia", "model_id": "nvidia/nemotron-3-ultra-550b-a55b",
                         "context": {"window": 1_000_000, "max_output_tokens": 182_520}},
    }
    assert matched([row], "nvidia/nemotron-3-ultra-550b-a55b")["provider_id"] == "nvidia"
    facts = normalize(row)
    assert facts["context_window"] == 1_000_000
    assert facts["max_output_tokens"] == 182_520


def test_normalize_inherits_the_model_document_by_default():
    facts = normalize(entry("gpt-4o"))
    assert facts["context_window"] == 1_000
    assert facts["max_output_tokens"] == 500
    assert facts["pricing"]["input"] == 1


def test_normalize_explicit_null_marks_the_fact_unknown_here():
    facts = normalize(entry("gpt-4o", pricing=None))
    assert facts["pricing"] is None
    assert facts["context_window"] == 1_000


def test_normalize_replaces_nested_objects_wholesale():
    facts = normalize(entry("gpt-4o", context={"window": 250}))
    assert facts["context_window"] == 250
    # Fields omitted inside a replacement are unknown, not inherited.
    assert facts["max_output_tokens"] is None


def test_normalize_maps_capabilities_onto_wire_keys():
    facts = normalize(entry(
        "gpt-4o",
        capabilities={"tool_use": True, "function_calling": True, "vision": False,
                      "reasoning": True, "structured_output": None},
    ))
    assert facts["capabilities"] == {
        "tools": True, "vision": False, "reasoning": True, "structured_output": None,
    }


def test_normalize_infers_vision_from_image_input():
    with_images = normalize(entry(
        "gpt-4o", modalities={"input": ["text", "image"], "output": ["text"]},
    ))
    assert with_images["capabilities"]["vision"] is True
    text_only = normalize(entry("gpt-4o"))
    assert text_only["capabilities"] is None


def test_provider_keys_only_ever_name_registered_ids():
    assert model_info.provider_keys("nvidia-nim") == ("nvidia",)
    assert model_info.provider_keys("gemini") == ("google",)
    assert model_info.provider_keys("opencode-zen") == ("opencode",)
    # A provider model-info does not catalogue is never guessed at.
    assert model_info.provider_keys("ollama") == ()


async def test_lookup_reads_a_local_catalog(tmp_path, monkeypatch):
    target = tmp_path / "v1" / "providers" / "openai"
    target.mkdir(parents=True)
    (target / "models.json").write_text(json.dumps([
        entry("gpt-4o"),
        entry("gpt-4.1", context={"window": 1_047_576, "max_output_tokens": 32_768}),
    ]))
    monkeypatch.setattr(config, "MODEL_INFO_URL", str(tmp_path / "v1"))

    facts = await model_info.lookup(("openai",), "gpt-4o")
    assert facts["context_window"] == 1_000
    facts = await model_info.lookup(("openai",), "gpt-4.1")
    assert facts["context_window"] == 1_047_576
    assert await model_info.lookup(("openai",), "unknown-model") == {}


async def test_lookup_tries_each_registered_provider_in_order(tmp_path, monkeypatch):
    for provider, rows in (("nvidia", []),
                           ("openrouter", [entry("deepseek/deepseek-v4-pro")])):
        target = tmp_path / "v1" / "providers" / provider
        target.mkdir(parents=True)
        (target / "models.json").write_text(json.dumps(rows))
    monkeypatch.setattr(config, "MODEL_INFO_URL", str(tmp_path / "v1"))

    facts = await model_info.lookup(("nvidia", "openrouter"), "deepseek-ai/deepseek-v4-pro")
    assert facts["context_window"] == 1_000


async def test_lookup_returns_nothing_when_the_catalog_is_unreachable(monkeypatch):
    monkeypatch.setattr(config, "MODEL_INFO_URL", "/path/that/does/not/exist")
    assert await model_info.lookup(("openai",), "gpt-4o") == {}
    assert "openai" not in model_info._catalog_cache


async def test_a_failed_refresh_serves_the_stale_catalog(monkeypatch):
    rows = [entry("gpt-4o")]
    model_info._catalog_cache["openai"] = {"value": rows, "fetched_at": 0.0}

    def unreachable(**kwargs):
        raise httpx.ConnectError("offline")

    monkeypatch.setattr(httpx, "AsyncClient", unreachable)
    monkeypatch.setattr(config, "MODEL_INFO_URL", "https://example.test/v1")
    assert (await model_info.lookup(("openai",), "gpt-4o"))["context_window"] == 1_000


async def test_the_http_catalog_is_downloaded_once_per_ttl(monkeypatch):
    calls = []
    original = httpx.AsyncClient

    def handle(request):
        calls.append(request.url.path)
        return httpx.Response(200, json=[entry("gpt-4o")])

    monkeypatch.setattr(httpx, "AsyncClient",
                        lambda **kw: original(transport=httpx.MockTransport(handle), **kw))
    monkeypatch.setattr(config, "MODEL_INFO_URL", "https://example.test/v1")

    first = await model_info.lookup(("openai",), "gpt-4o")
    second = await model_info.lookup(("openai",), "gpt-4o")
    assert first["context_window"] == second["context_window"] == 1_000
    assert calls == ["/v1/providers/openai/models.json"]
    assert model_info._catalog_cache["openai"]["fetched_at"] <= time.monotonic()
