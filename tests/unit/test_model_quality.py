from mix_agent.providers.model_quality import quality_prior, request_cost


def test_quality_prior_recognizes_families_and_sizes():
    assert quality_prior({"model_id": "gpt-4.1"}) > quality_prior({"model_id": "gpt-4o-mini"})
    assert quality_prior({"model_id": "meta-llama/Llama-3.1-70B-Instruct"}) > quality_prior(
        {"model_id": "meta-llama/Llama-3.1-8B-Instruct"}
    )
    assert quality_prior({"model_id": "totally-unknown-model"}) is None
    assert quality_prior({}) is None


def test_quality_prior_reasoning_bonus():
    base = quality_prior({"model_id": "o1"})
    boosted = quality_prior({"model_id": "o1", "capabilities": {"reasoning": True}})
    assert boosted == min(1.0, base + 0.04)


def test_request_cost_uses_manual_and_metadata_prices():
    manual = request_cost({"pricing_override": {"input": 10.0, "output": 20.0}}, 1000, 1000)
    assert manual == round(1000 / 1_000_000 * 10.0 + 1000 / 1_000_000 * 20.0, 8)
    assert request_cost({}, 1000, 1000) is None
    metadata = request_cost({"metadata": {"pricing": {"value": {"input": 5.0, "output": 5.0}}}}, 1000, 0)
    assert metadata == round(1000 / 1_000_000 * 5.0, 8)
