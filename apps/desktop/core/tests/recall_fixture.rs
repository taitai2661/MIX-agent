//! The ranking contract both editions are held to, read from one shared file.
//!
//! `tests/fixtures/memory_recall_cases.json` states each case's inputs and
//! the exact score every candidate must produce. The Docker edition's
//! `tests/unit/test_memory_ranking.py` asserts those same numbers, so a
//! weighting change in either implementation surfaces as a specific number
//! that moved rather than as "the order looked different".
//!
//! The fixture is duplicated into each edition and the copies are held
//! byte-identical by `scripts/check-memory-parity.py`, exactly like the spec
//! itself: each repository stays cloneable on its own, and CI — which checks
//! out both — is what notices drift. Each case also supplies its own `terms`,
//! so neither edition's tokeniser can make this test pass or fail on its own.
//!
//! Reading and scoring live in `mixd_core::recall_fixture` so
//! `src/bin/retrieval_bench.rs` measures against the very same interpretation
//! of the file; this test is the half that *asserts* the numbers.

use chrono::Utc;
use mixd_core::recall_fixture;
use mixd_core::retrieval::{vector_score, MIN_SCORE, RESULT_LIMIT_DEFAULT};

/// The fixture's numbers are written to four decimals, as the
/// implementations round them; compare in that same precision rather than as
/// raw `f64`s, which would make equality depend on the last bit.
fn four(value: f64) -> String {
    format!("{value:.4}")
}

#[test]
fn every_case_scores_exactly_as_the_shared_fixture_says() {
    let raw = recall_fixture::load_value().expect("fixture loads");
    let now = Utc::now();
    let cases = raw["cases"].as_array().expect("fixture has cases");
    assert!(!cases.is_empty(), "the contract must have cases to check");

    for case in cases {
        let id = case["id"].as_str().expect("case id");
        let description = case["description"].as_str().unwrap_or("");
        // The contract test always scores with the case's own vector — the
        // bench's lexical/hybrid split is the bench's knob, not this test's.
        let query_vector = recall_fixture::optional_vector(&case["query_vector"])
            .expect("case query_vector parses");
        let scored = recall_fixture::rank(case, query_vector.as_deref(), now, RESULT_LIMIT_DEFAULT);

        let scores: serde_json::Map<String, serde_json::Value> =
            case["scores"].as_object().expect("case scores").clone();
        let ranked: Vec<&str> = scored.iter().map(|item| item.id.as_str()).collect();

        assert_eq!(
            scored.len(),
            scores.len(),
            "case `{id}` kept the wrong number of candidates: {description}\n got {ranked:?}\nwant {:?}",
            scores.keys().collect::<Vec<_>>()
        );

        for item in &scored {
            let expected = scores
                .get(&item.id)
                .unwrap_or_else(|| panic!("case `{id}` did not expect `{}`", item.id));
            assert_eq!(
                four(item.score),
                four(expected.as_f64().expect("expected score")),
                "case `{id}` / `{}`: {description}",
                item.id
            );
        }

        match case["expected"].as_str() {
            None => assert!(
                scored.is_empty(),
                "case `{id}` must return nothing: {description}"
            ),
            Some(expected) => assert_eq!(
                ranked.first().copied(),
                Some(expected),
                "case `{id}` picked the wrong winner: {description}"
            ),
        }
    }
}

/// Spec §5.2: the fixture repeats the weight block so it can be read on its
/// own. This catches the case where the Rust constants drift away from both.
#[test]
fn the_fixture_repeats_the_constants_this_edition_uses() {
    let raw = recall_fixture::load_value().expect("fixture loads");
    let declared = raw["ranking"].as_object().expect("ranking block");

    assert_eq!(
        declared["lexical_weight_with_vector"].as_f64(),
        Some(mixd_core::retrieval::LEXICAL_WEIGHT_WITH_VECTOR)
    );
    assert_eq!(
        declared["vector_weight_with_vector"].as_f64(),
        Some(mixd_core::retrieval::VECTOR_WEIGHT_WITH_VECTOR)
    );
    assert_eq!(
        declared["recency_weight_with_vector"].as_f64(),
        Some(mixd_core::retrieval::RECENCY_WEIGHT_WITH_VECTOR)
    );
    assert_eq!(
        declared["lexical_weight_without_vector"].as_f64(),
        Some(mixd_core::retrieval::LEXICAL_WEIGHT_WITHOUT_VECTOR)
    );
    assert_eq!(
        declared["recency_weight_without_vector"].as_f64(),
        Some(mixd_core::retrieval::RECENCY_WEIGHT_WITHOUT_VECTOR)
    );
    assert_eq!(declared["min_score"].as_f64(), Some(MIN_SCORE));
}

/// The two fixture dimensions the implementations are most likely to disagree
/// about: a mismatched dimension must mean "no vector", never "zero
/// similarity".
#[test]
fn a_dimension_mismatch_is_an_absent_vector_not_a_zero() {
    // Orthogonal on the same axis count: comparable, score 0.5.
    assert_eq!(vector_score(&[1.0, 0.0], &[0.0, 1.0]), Some(0.5));
    // Different lengths: incomparable, so the pair falls back to lexical.
    assert_eq!(vector_score(&[1.0, 0.0], &[1.0, 0.0, 0.0]), None);
    // An empty side has no direction either.
    assert_eq!(vector_score(&[], &[]), None);
}
