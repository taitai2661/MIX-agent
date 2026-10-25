//! Integration tests for Memory retrieval, memory spec §5
//! (`docs/memory-spec.md`).
//!
//! Phase 1 added the `vector` term to Memory ranking, and the contract that
//! matters more than the new hits is the one about *not* breaking the old
//! ones: every way of lacking a usable query embedding must resolve to the
//! lexical ranking these tests also exercise. Both a provider that has no
//! embeddings endpoint and one whose vectors are unusable are therefore
//! asserted to return the same memories in the same order.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use mixd_core::retrieval::{
    cosine_similarity, recency_score, retrieve_memories, retrieve_memories_hybrid, EmbeddingSource,
    MIN_SCORE, RESULT_LIMIT_MAX,
};
use mixd_providers::{Provider, ProviderCapabilities, ProviderId, ProviderRequest, StreamBox};
use mixd_storage::Storage;

/// The model every fixture row names beside its vector. Vectors are only
/// compared inside one model family (spec §2.2), so a single id keeps the
/// fixtures about dimensions and directions rather than model bookkeeping.
const MODEL: &str = "stub-2d";

/// A provider that answers `embed` from a canned table.
///
/// `stream` is never used here — retrieval must not need a chat turn — so it
/// fails loudly rather than silently producing an empty answer.
struct StubEmbedder {
    name: String,
    vectors: HashMap<String, Vec<f64>>,
}

impl StubEmbedder {
    fn new(vectors: &[(&str, Vec<f64>)]) -> Self {
        Self {
            name: "stub-embedder".to_string(),
            vectors: vectors
                .iter()
                .map(|(text, vector)| (text.to_string(), vector.clone()))
                .collect(),
        }
    }
}

#[async_trait]
impl Provider for StubEmbedder {
    fn id(&self) -> &ProviderId {
        &self.name
    }
    fn label(&self) -> &str {
        "stub embedder"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: false,
            tool_calling: false,
            reasoning: false,
            streaming: false,
        }
    }
    async fn stream(&self, _req: ProviderRequest) -> anyhow::Result<StreamBox> {
        anyhow::bail!("the retrieval tests never stream")
    }
    async fn embed(&self, _model: &str, texts: &[String]) -> anyhow::Result<Vec<Vec<f64>>> {
        texts
            .iter()
            .map(|text| {
                self.vectors
                    .get(text)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no canned vector for {text:?}"))
            })
            .collect()
    }
}

/// A provider with no embeddings endpoint, like Anthropic: it keeps the
/// trait's default `embed`, which answers `EmbeddingNotSupported`.
struct NoEmbedProvider {
    name: String,
}

impl Default for NoEmbedProvider {
    fn default() -> Self {
        Self {
            name: "no-embed".to_string(),
        }
    }
}

#[async_trait]
impl Provider for NoEmbedProvider {
    fn id(&self) -> &ProviderId {
        &self.name
    }
    fn label(&self) -> &str {
        "provider without embeddings"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: false,
            tool_calling: false,
            reasoning: false,
            streaming: false,
        }
    }
    async fn stream(&self, _req: ProviderRequest) -> anyhow::Result<StreamBox> {
        anyhow::bail!("the retrieval tests never stream")
    }
    // `embed` is deliberately not overridden: the default is what a transport
    // with no embeddings endpoint answers, and that is the path under test.
}

async fn temp_storage() -> Storage {
    let dir = std::env::temp_dir().join(format!("mixd-retrieval-test-{}", uuid::Uuid::new_v4()));
    Storage::open(&dir).await.expect("open temp storage")
}

/// Insert one retrievable Memory row with an explicit `updated_at`, so recency
/// is a fixed input of the test rather than a race against the clock.
async fn insert_memory(
    storage: &Storage,
    id: &str,
    title: &str,
    content: &str,
    updated_at: &str,
    embedding: Option<Vec<f64>>,
) {
    let mut data = serde_json::json!({ "title": title, "content": content });
    if let Some(values) = embedding {
        data["embedding"] = serde_json::json!(values);
        data["embedding_model"] = serde_json::json!(MODEL);
    }
    sqlx::query(
        "INSERT INTO memories (id, owner_id, data, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(data.to_string())
    .bind(updated_at)
    .bind(updated_at)
    .execute(storage.pool())
    .await
    .expect("insert memory");
}

fn ids(hits: &[mixd_core::retrieval::MemoryHit]) -> Vec<&str> {
    hits.iter().map(|hit| hit.id.as_str()).collect()
}

/// Spec §5.2's promise that a vector is additive: a query phrased so that it
/// shares no word with the memory it means still returns that memory, while
/// the lexical-only entry point — what the app served before Phase 1 —
/// returns nothing at all for the same query.
#[tokio::test]
async fn a_query_sharing_no_words_with_a_memory_finds_it_through_the_vector() {
    let storage = temp_storage().await;
    // Identical timestamps: recency must not be what decides this test.
    let updated = Utc::now().to_rfc3339();
    insert_memory(
        &storage,
        "dog",
        "Pochi",
        "The family dog answers to Pochi.",
        &updated,
        Some(vec![1.0, 0.0]),
    )
    .await;
    insert_memory(
        &storage,
        "groceries",
        "Shopping",
        "milk eggs bread",
        &updated,
        Some(vec![0.0, 1.0]),
    )
    .await;
    // No vector at all: even with a query embedding available this row must
    // stay out, because it matches on neither axis.
    insert_memory(
        &storage,
        "sky",
        "Weather",
        "the sky is blue",
        &updated,
        None,
    )
    .await;

    // None of these three terms occurs in any row above.
    let query = "canine companion moniker";

    let lexical = retrieve_memories(&storage, query, 8)
        .await
        .expect("lexical retrieval");
    assert!(
        lexical.is_empty(),
        "the fixture is built to have no word overlap, got {:?}",
        ids(&lexical)
    );

    let source = EmbeddingSource::new(
        Arc::new(StubEmbedder::new(&[(query, vec![1.0, 0.0])])),
        MODEL,
    );
    let hybrid = retrieve_memories_hybrid(&storage, query, 8, Some(&source))
        .await
        .expect("hybrid retrieval");

    // The dog's vector agrees with the query's, the groceries' is orthogonal,
    // and the unembedded row is ranked by neither: exactly one match on each
    // side, ordered by the vector term.
    assert_eq!(ids(&hybrid), vec!["dog", "groceries"]);
    assert!(hybrid[0].score > hybrid[1].score);
}

/// Spec §2.2: `without_vector` has to reproduce the pre-Phase-1 ranking.
///
/// The legacy score is written out inline below — relevance weighted 0.6 plus
/// the row's salience, strength and confidence — with every row inserted at
/// its default salience / strength / confidence (0.5 / 0.6 / 0.7), so those
/// three constants contribute the same amount to every candidate and the
/// order is decided by relevance alone. Both the lexical entry point and the
/// hybrid one, handed a provider that cannot embed, must return that order.
#[tokio::test]
async fn rows_without_embeddings_rank_the_same_for_a_provider_that_cannot_embed() {
    fn legacy_score(lexical: f64) -> f64 {
        const SALIENCE: f64 = 0.5;
        const STRENGTH: f64 = 0.6;
        const CONFIDENCE: f64 = 0.7;
        0.6 * lexical + 0.15 * SALIENCE + 0.15 * STRENGTH + 0.1 * CONFIDENCE
    }

    let storage = temp_storage().await;
    // One fixed timestamp: recency is the same constant for every row, so the
    // comparison is about the lexical term only.
    let updated = "2024-05-01T00:00:00Z";
    // Deliberately out of relevance order, to show the ranking sorts rather
    // than echoing insertion order.
    insert_memory(&storage, "two", "Pair", "alpha bravo", updated, None).await;
    insert_memory(&storage, "none", "Unrelated", "zulu yankee", updated, None).await;
    insert_memory(
        &storage,
        "all",
        "Trio",
        "alpha bravo charlie",
        updated,
        None,
    )
    .await;
    insert_memory(&storage, "one", "Single", "alpha", updated, None).await;

    let query = "alpha bravo charlie";
    let mut expected: Vec<(&str, f64)> =
        vec![("all", 3.0 / 3.0), ("two", 2.0 / 3.0), ("one", 1.0 / 3.0)];
    expected.sort_by(|a, b| {
        legacy_score(b.1)
            .partial_cmp(&legacy_score(a.1))
            .expect("comparable scores")
    });
    let expected: Vec<&str> = expected.into_iter().map(|(id, _)| id).collect();

    let lexical = retrieve_memories(&storage, query, 8)
        .await
        .expect("lexical retrieval");
    assert_eq!(ids(&lexical), expected);

    let source = EmbeddingSource::new(Arc::new(NoEmbedProvider::default()), MODEL);
    let hybrid = retrieve_memories_hybrid(&storage, query, 8, Some(&source))
        .await
        .expect("hybrid retrieval with a non-embedding provider");
    assert_eq!(
        ids(&hybrid),
        ids(&lexical),
        "a provider that cannot embed must not change the ranking"
    );
    // The row with no matching vocabulary stays out of both, and it is the
    // only row the gate (not the score) is responsible for.
    assert!(!ids(&hybrid).contains(&"none"));
}

/// Spec §2 `min_score`: a candidate whose weighted sum falls below the floor
/// is not worth a slot in the result.
///
/// Both rows match exactly one term of a ten-term query, so they clear the
/// lexical gate identically; the only thing that can separate them is the
/// score, which puts the decades-old row under `MIN_SCORE` and the fresh one
/// well over it.
#[tokio::test]
async fn a_candidate_scoring_below_the_minimum_is_not_returned() {
    let storage = temp_storage().await;
    let ancient = "1970-01-01T00:00:00Z";
    // Now rather than a fixed date: the point is a row the reader considers
    // fresh, which stays true however far the calendar has moved.
    let fresh = Utc::now().to_rfc3339();
    insert_memory(&storage, "old", "Old fact", "alpha", ancient, None).await;
    insert_memory(&storage, "recent", "Fresh fact", "alpha", &fresh, None).await;

    let query = "alpha bravo charlie delta echo foxtrot golf hotel india juliet";
    // Ten distinct terms, of which each row matches exactly one: lexical 0.1
    // for both, so the gate passes and only `MIN_SCORE` can drop a row.
    let now = Utc::now();
    let ancient_score = 0.85 * 0.1 + 0.15 * recency_score(ancient, now);
    let fresh_score = 0.85 * 0.1 + 0.15 * recency_score(&fresh, now);
    assert!(
        ancient_score < MIN_SCORE,
        "the fixture must be below the floor, got {ancient_score}"
    );
    assert!(fresh_score >= MIN_SCORE, "got {fresh_score}");

    let source = EmbeddingSource::new(Arc::new(NoEmbedProvider::default()), MODEL);
    let hits = retrieve_memories_hybrid(&storage, query, 8, Some(&source))
        .await
        .expect("hybrid retrieval");
    assert_eq!(ids(&hits), vec!["recent"]);
}

/// Spec §2 `result_limit_max`: asking for more than the maximum returns the
/// maximum, never an unbounded list. (The `memory` tool used to clamp to 25
/// while the spec says 30 — see `mixd_tools::memory`.)
#[tokio::test]
async fn a_limit_above_the_maximum_is_clamped_to_it() {
    let storage = temp_storage().await;
    let updated = Utc::now().to_rfc3339();
    for index in 0..(RESULT_LIMIT_MAX + 10) {
        insert_memory(
            &storage,
            &format!("row-{index:02}"),
            "Fact",
            "alpha",
            &updated,
            None,
        )
        .await;
    }

    let source = EmbeddingSource::new(Arc::new(NoEmbedProvider::default()), MODEL);
    let overasked = retrieve_memories_hybrid(&storage, "alpha", 500, Some(&source))
        .await
        .expect("hybrid retrieval");
    assert_eq!(overasked.len(), RESULT_LIMIT_MAX);

    // The lexical entry point clamps by the same rule.
    let lexical = retrieve_memories(&storage, "alpha", 500)
        .await
        .expect("lexical retrieval");
    assert_eq!(lexical.len(), RESULT_LIMIT_MAX);
}

/// A row stored with a different vector dimension than the query's is not
/// comparable, and spec §2.2 routes such a pair to the `without_vector`
/// weights instead of failing, panicking, or pretending the vectors agree.
#[tokio::test]
async fn a_cosine_dimension_mismatch_falls_back_to_lexical_ranking() {
    let updated = "2026-01-01T00:00:00Z";
    let storage = temp_storage().await;
    insert_memory(
        &storage,
        "mismatch",
        "Trio",
        "alpha bravo charlie",
        updated,
        // Three dimensions against the query's two.
        Some(vec![0.3, 0.4, 0.5]),
    )
    .await;

    let query = "alpha bravo charlie";
    let query_vector = vec![1.0, 0.0];
    assert_eq!(cosine_similarity(&query_vector, &[0.3, 0.4, 0.5]), None);

    let source = EmbeddingSource::new(
        Arc::new(StubEmbedder::new(&[(query, query_vector.clone())])),
        MODEL,
    );
    let hits = retrieve_memories_hybrid(&storage, query, 8, Some(&source))
        .await
        .expect("hybrid retrieval");
    assert_eq!(ids(&hits), vec!["mismatch"]);

    // Scored with the lexical-only weights, exactly as an unembedded row
    // would be: the mismatch must not cost the row its lexical relevance.
    let expected = 0.85 * 1.0 + 0.15 * recency_score(updated, Utc::now());
    assert!(
        (hits[0].score - expected).abs() < 1e-6,
        "expected the without_vector score {expected}, got {}",
        hits[0].score
    );
}

/// The vector term, when it *is* comparable, has to actually move a row up.
///
/// Orthogonal vectors score 0.5 on the `(1 + cosine) / 2` mapping, so the
/// matching row wins by the full `vector_weight_with_vector` margin rather
/// than by a tie-break.
#[tokio::test]
async fn the_vector_term_decides_between_equally_lexical_rows() {
    let updated = Utc::now().to_rfc3339();
    let storage = temp_storage().await;
    insert_memory(
        &storage,
        "near",
        "Semantic match",
        "a row whose vector agrees with the query",
        &updated,
        Some(vec![1.0, 0.0]),
    )
    .await;
    insert_memory(
        &storage,
        "far",
        "Semantic miss",
        "a row whose vector is orthogonal to the query",
        &updated,
        Some(vec![0.0, 1.0]),
    )
    .await;

    let query = "canine companion moniker";
    let source = EmbeddingSource::new(
        Arc::new(StubEmbedder::new(&[(query, vec![1.0, 0.0])])),
        MODEL,
    );
    let hits = retrieve_memories_hybrid(&storage, query, 8, Some(&source))
        .await
        .expect("hybrid retrieval");
    assert_eq!(ids(&hits), vec!["near", "far"]);

    // Both rows cleared the floor on the vector term alone, which is the
    // half of §5.2 that lets a row with no matching vocabulary in.
    for hit in &hits {
        assert!(hit.score >= MIN_SCORE, "{} scored {}", hit.id, hit.score);
    }
}
