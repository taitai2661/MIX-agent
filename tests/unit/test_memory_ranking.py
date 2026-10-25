"""The hybrid ranking contract, driven by the shared recall fixture.

``tests/fixtures/memory_recall_cases.json`` lives in the Docker edition but is
the contract for both: the Desktop edition asserts the same cases from Rust.
Sharing one file keeps the two editions honest without sharing code, and pinning
an exact score per candidate means a weighting change shows up as a specific
number instead of as "something moved".
"""

import json
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from mix_agent.memory import embeddings
from mix_agent.memory import service

FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "memory_recall_cases.json"
CASES = json.loads(FIXTURE.read_text(encoding="utf-8"))["cases"]


def _candidates(case):
    now = datetime.now(UTC)
    return [
        (
            set(memory["terms"]),
            now - timedelta(days=memory["age_days"]),
            memory.get("vector"),
            memory["id"],
        )
        for memory in case["memories"]
    ]


def _scores(case):
    ranked = service.rank(
        set(case["query_terms"]),
        _candidates(case),
        query_vector=case.get("query_vector"),
    )
    return ranked, {item["payload"]: item["relevance"] for item in ranked}


@pytest.mark.parametrize("case", CASES, ids=lambda case: case["id"])
def test_ranking_matches_the_shared_fixture(case):
    ranked, scores = _scores(case)

    assert scores == case["scores"], case["description"]
    if case["expected"] is None:
        assert ranked == []
    else:
        assert ranked[0]["payload"] == case["expected"], case["description"]


def test_a_query_without_terms_ranks_nothing():
    assert service.rank(set(), [("term", datetime.now(UTC), None, "row")]) == []


def test_limit_caps_the_ranked_set():
    now = datetime.now(UTC)
    candidates = [(set("row"), now - timedelta(days=index), None, f"row-{index}") for index in range(10)]
    ranked = service.rank({"row"}, candidates, limit=3)

    assert [item["payload"] for item in ranked] == ["row-0", "row-1", "row-2"]


def test_ranking_constants_match_the_spec_block():
    """The fixture repeats the spec's numbers so the test is self-contained; this
    catches the case where the code drifts away from both."""
    declared = json.loads(FIXTURE.read_text(encoding="utf-8"))["ranking"]
    actual = {
        "lexical_weight_with_vector": service.LEXICAL_WEIGHT_WITH_VECTOR,
        "vector_weight_with_vector": service.VECTOR_WEIGHT_WITH_VECTOR,
        "recency_weight_with_vector": service.RECENCY_WEIGHT_WITH_VECTOR,
        "lexical_weight_without_vector": service.LEXICAL_WEIGHT_WITHOUT_VECTOR,
        "recency_weight_without_vector": service.RECENCY_WEIGHT_WITHOUT_VECTOR,
        "min_score": service.MIN_SCORE,
    }
    assert actual == declared


@pytest.mark.parametrize(
    ("left", "right", "expected"),
    [
        ([1.0, 0.0], [1.0, 0.0], 1.0),
        ([1.0, 0.0], [0.0, 1.0], 0.5),
        ([1.0, 0.0], [-1.0, 0.0], 0.0),
        ([1.0, 0.0], [1.0, 0.0, 0.0], None),      # dimension mismatch is not a score
        ([1.0, 0.0], [], None),
        ([], [1.0, 0.0], None),
        ([0.0, 0.0], [0.0, 0.0], None),           # a zero vector has no direction
        ([1.0, 0.0], [float("nan"), 0.0], None),  # nor does a poisoned one
    ],
)
def test_cosine_is_normalized_or_absent(left, right, expected):
    assert embeddings.cosine01(left, right) == expected


@pytest.mark.parametrize(
    ("data", "usable"),
    [
        ({"embedding": [0.1] * 8}, True),
        ({"embedding": [0.1] * 4}, False),  # too short to be a real embedding
        ({"embedding": "not-a-vector"}, False),
        ({"embedding": []}, False),
        ({"embedding": [0.1] * 7 + [float("inf")]}, False),
        ({}, False),
        (None, False),
    ],
)
def test_row_vector_rejects_unusable_payloads(data, usable):
    assert (embeddings.row_vector(data) is not None) is usable


def test_stale_vectors_are_detected_per_model():
    fresh = {"embedding": [0.1] * 8, "embedding_model": "text-embedding-3-small"}

    assert embeddings.is_stale(fresh, "text-embedding-3-small") is False
    assert embeddings.is_stale(fresh, "other-model") is True
    assert embeddings.is_stale(fresh, "") is False  # no model to compare against
    assert embeddings.is_stale({}, "text-embedding-3-small") is True


ANTHROPIC = {"id": "prov-1", "kind": "anthropic", "base_url": None, "data": {}}


@pytest.mark.asyncio
async def test_an_unsupported_provider_fails_softly():
    """Anthropic exposes no embeddings, so every entry point must report that
    as \"no vector\" rather than raising into a request or a tool call."""
    from mix_agent.providers.adapters import Adapter, EmbeddingNotSupported

    with pytest.raises(EmbeddingNotSupported):
        await Adapter(ANTHROPIC, "key").embed(["hello"])

    assert await embeddings.embed_texts(ANTHROPIC, "key", ["hello"]) is None
    assert await embeddings.embed_content(ANTHROPIC, "key", "hello") is None
    assert await embeddings.query_vector(ANTHROPIC, "key", "hello") is None


@pytest.mark.asyncio
async def test_blank_input_never_reaches_the_provider():
    assert await embeddings.query_vector(ANTHROPIC, "key", "   ") is None
    assert await embeddings.query_vector(None, "key", "hello") is None
    assert await embeddings.embed_texts(ANTHROPIC, "key", []) is None


@pytest.mark.asyncio
async def test_query_vectors_are_cached():
    embeddings.clear_query_cache()
    calls = []

    class FakeAdapter:
        def __init__(self, provider, key):
            self.provider = provider

        async def embed(self, items):
            calls.append(items)
            return [[0.5] * 8 for _ in items]

    import mix_agent.memory.embeddings as module

    original = module._embed_batch

    async def fake(provider, key, texts):
        items = [str(text).strip() for text in texts if str(text or "").strip()]
        if not items:
            return None
        adapter = FakeAdapter(provider, key)
        return ("fake-model", await adapter.embed(items))

    module._embed_batch = fake
    try:
        first = await embeddings.query_vector(ANTHROPIC, "key", "the same question")
        second = await embeddings.query_vector(ANTHROPIC, "key", "the same  question")
    finally:
        module._embed_batch = original
        embeddings.clear_query_cache()

    assert first == [0.5] * 8
    assert second == first  # whitespace-normalised, so the same cache entry
    assert len(calls) == 1  # one provider call, not two


def test_store_writes_the_vector_and_the_model():
    stored = embeddings.store({"content": "hello"}, "model-a", [0.25, 0.5])

    assert stored["embedding"] == [0.25, 0.5]
    assert stored["embedding_model"] == "model-a"
    assert stored["content"] == "hello"  # unrelated keys survive
