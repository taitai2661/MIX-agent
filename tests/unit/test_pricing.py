from mix_agent import pricing
from mix_agent.usage import token_usage


def test_normalize_model_info_pricing():
    assert pricing.normalize({"currency": "USD", "unit": "1M_tokens",
                              "input": 3, "output": 15, "cached_input": 0.3, "cached_write": 3.75}) == {
        "input": 3.0, "output": 15.0, "cache_read": 0.3, "cache_write": 3.75,
    }


def test_normalize_converts_stated_units():
    assert pricing.normalize({"unit": "1K_tokens", "input": 0.003, "output": 0.015}) == {
        "input": 3.0, "output": 15.0, "cache_read": None, "cache_write": None,
    }
    assert pricing.normalize({"unit": "1_token", "input": 0.000003, "output": 0.000015}) == {
        "input": 3.0, "output": 15.0, "cache_read": None, "cache_write": None,
    }


def test_normalize_rejects_unusable_units_and_currencies():
    # Prices are only estimated in USD per million tokens; anything else is
    # unknown rather than silently converted.
    assert pricing.normalize({"currency": "JPY", "input": 100, "output": 300}) is None
    assert pricing.normalize({"unit": "per_request", "input": 1, "output": 2}) is None


def test_normalize_keeps_explicit_zero_prices():
    # An explicit zero is a real (free) price and must stay distinguishable
    # from an unknown price.
    assert pricing.normalize({"input": 0, "output": 0}) == {
        "input": 0.0, "output": 0.0, "cache_read": None, "cache_write": None,
    }


def test_normalize_rejects_unusable_rows():
    assert pricing.normalize({}) is None
    assert pricing.normalize(None) is None
    assert pricing.normalize("free") is None
    assert pricing.normalize({"input": -1, "output": "cheap"}) is None


def test_normalize_accepts_alias_keys():
    prices = pricing.normalize({"prompt": 1, "completion": 2})
    assert prices["input"] == 1.0
    assert prices["output"] == 2.0


def test_resolve_prefers_manual_override():
    data = {
        "metadata": {"pricing": {"value": {"input": 1, "output": 1}, "source": "model_info"}},
        "pricing_override": {"input": 5, "output": 10},
    }
    prices, source = pricing.resolve(data)
    assert source == "manual"
    assert prices["input"] == 5.0


def test_resolve_uses_catalog_evidence():
    data = {"metadata": {"pricing": {"value": {"input": 2, "output": 4}, "source": "model_info"}}}
    prices, source = pricing.resolve(data)
    assert source == "model_info"
    assert prices["output"] == 4.0


def test_resolve_defaults_to_model_info_source():
    data = {"metadata": {"pricing": {"value": {"input": 2, "output": 4}}}}
    assert pricing.resolve(data)[1] == "model_info"


def test_resolve_unknown_without_prices():
    assert pricing.resolve({}) == (None, "unknown")
    assert pricing.resolve(None) == (None, "unknown")


def test_estimate_applies_each_price():
    prices = {"input": 3, "output": 15, "cache_read": 0.3, "cache_write": None}
    assert pricing.estimate(prices, 1_000_000, 1_000_000, 0) == 18.0
    assert pricing.estimate(prices, 1_000_000, 0, 1_000_000) == 0.3
    # Without a cache rate, cached input is priced as regular input.
    assert pricing.estimate({"input": 2, "output": 2}, 1_000_000, 0, 1_000_000) == 2.0
    assert pricing.estimate(None, 10, 10, 0) is None


def test_token_usage_normalizes_provider_shapes():
    openai = token_usage({
        "prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14,
        "prompt_tokens_details": {"cached_tokens": 6},
    })
    assert openai == {
        "input_tokens": 10, "output_tokens": 4, "cached_tokens": 6,
        "reasoning_tokens": 0, "total_tokens": 14,
    }
    anthropic = token_usage({"input_tokens": 10, "output_tokens": 4, "cache_read_input_tokens": 6})
    assert anthropic["cached_tokens"] == 6
    assert anthropic["total_tokens"] == 14
    gemini = token_usage({"promptTokenCount": 10, "candidatesTokenCount": 4, "totalTokenCount": 14})
    assert gemini["input_tokens"] == 10


def test_token_usage_ignores_empty_and_unknown():
    assert token_usage({}) is None
    assert token_usage(None) is None
    assert token_usage({"unrelated": 1}) is None
