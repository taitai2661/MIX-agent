//! The single reader and scorer for the ranking contract both editions share.
//!
//! `tests/fixtures/memory_recall_cases.json` states each case's inputs and the
//! exact score every candidate must produce. Two programs care about it:
//! `tests/recall_fixture.rs` asserts those numbers, and
//! `src/bin/retrieval_bench.rs` measures retrieval quality against the same
//! inputs. Both go through this module so the fixture is parsed in exactly one
//! place — a second reader would be a second interpretation of the contract,
//! and the two would drift at the first field rename.
//!
//! The scoring here is spec §5's own arithmetic as `crate::retrieval` applies
//! it: §5.1's terms, §5.2's per-pair weights (via
//! [`crate::retrieval::rank_score`]), §5.2's `min_score` gate, and §5.2's
//! tie-break (newest first). It runs over the fixture's *supplied* `terms`
//! rather than the production tokeniser — the fixture is about the ranking
//! formula, not about splitting text, which is also why neither edition's
//! tokeniser can make it pass or fail on its own.
//!
//! Fallible by design: every reader returns `anyhow::Result` with the fixture
//! path / case id / field in the message instead of panicking, so a damaged
//! fixture fails the bench and the contract test with a location, never with
//! an `expect` line number.

use std::fs;
use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};

use crate::retrieval::{rank_score, recency_at, vector_score, MIN_SCORE};

/// This edition's own copy of the shared fixture.
pub const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/memory_recall_cases.json"
);

/// Read the fixture as one JSON value.
///
/// One reader, one interpretation: the bench and the contract test both start
/// here instead of each growing their own idea of what the file says.
pub fn load_value() -> anyhow::Result<serde_json::Value> {
    let path = Path::new(FIXTURE);
    let raw = fs::read_to_string(path)
        .with_context(|| format!("cannot read shared fixture at {FIXTURE}"))?;
    serde_json::from_str(&raw).context("shared fixture is not valid JSON")
}

/// A string array, as every `terms` field in the fixture is stated.
pub fn terms(value: &serde_json::Value) -> anyhow::Result<Vec<String>> {
    let array = value.as_array().context("terms is not an array")?;
    array
        .iter()
        .enumerate()
        .map(|(i, item)| {
            item.as_str()
                .with_context(|| format!("terms[{i}] is not a string"))
                .map(str::to_string)
        })
        .collect()
}

/// An optional vector: absent or empty means "no vector for this side", which
/// is exactly what routes a pair to spec §2.2's `without_vector` weights.
///
/// A present-but-malformed vector is an error (with its index), never a
/// silent skip: silently dropping one dimension would re-weight the pair.
pub fn optional_vector(value: &serde_json::Value) -> anyhow::Result<Option<Vec<f64>>> {
    let Some(array) = value.as_array() else {
        return Ok(None);
    };
    if array.is_empty() {
        return Ok(None);
    }
    array
        .iter()
        .enumerate()
        .map(|(i, n)| {
            n.as_f64()
                .with_context(|| format!("vector[{i}] is not a number"))
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .map(Some)
}

/// One candidate that survived §5.2's gate, in rank order.
#[derive(Debug, Clone)]
pub struct Scored {
    pub id: String,
    pub score: f64,
    pub updated_at: DateTime<Utc>,
}

/// The candidate ids a case states — the universe a `must_not_match` set is
/// cut from (the bench's fixture suite scores noise against exactly this).
pub fn candidate_ids(case: &serde_json::Value) -> anyhow::Result<Vec<String>> {
    let memories = case["memories"]
        .as_array()
        .context("case has no memories array")?;
    memories
        .iter()
        .enumerate()
        .map(|(i, memory)| {
            memory["id"]
                .as_str()
                .with_context(|| format!("memories[{i}] has no string id"))
                .map(str::to_string)
        })
        .collect()
}

/// Spec §5.1's lexical term: overlap measured against the *query*, so a memory
/// that mentions a seventh of the query's words does not score lower merely
/// for being wordier.
fn lexical(query: &[String], row: &[String]) -> f64 {
    if query.is_empty() {
        return 0.0;
    }
    let overlap = query.iter().filter(|term| row.contains(term)).count();
    overlap as f64 / query.len().max(1) as f64
}

/// Spec §5.2's gate: an exact lexical hit survives a low combined score, but a
/// wholly unrelated query returns nothing rather than the least unrelated rows.
fn passes_gate(value: f64, overlap: bool) -> bool {
    value >= MIN_SCORE || overlap
}

/// Rank one case's candidates the way spec §5 ranks them.
///
/// `query_vector` is the caller's half of spec §2.2's per-pair choice:
/// `None` scores every pair with the `without_vector` weights, `Some` lets a
/// candidate that carries its own vector take the `with_vector` weights. The
/// contract test passes the case's own vector; the bench's `lexical` mode
/// passes `None` and its `hybrid` mode passes the case's vector.
///
/// The result is §5.2 in order: candidates under `min_score` without even a
/// single overlapping term are dropped, a case whose best candidate still
/// scores under `min_score` yields nothing at all, ties go to the more
/// recently updated row, and `limit` caps the list.
///
/// Malformed rows are skipped with a warn (counted in logs) rather than
/// panicking: one bad candidate must not fail the whole bench run.
pub fn rank(
    case: &serde_json::Value,
    query_vector: Option<&[f64]>,
    now: DateTime<Utc>,
    limit: usize,
) -> Vec<Scored> {
    let case_id = case["id"].as_str().unwrap_or("?");
    let query = match terms(&case["query_terms"]) {
        Ok(q) => q,
        Err(err) => {
            tracing::warn!(case_id, %err, "recall fixture case has no query_terms; scoring nothing");
            return Vec::new();
        }
    };
    let memories = match case["memories"].as_array() {
        Some(m) => m,
        None => {
            tracing::warn!(
                case_id,
                "recall fixture case has no memories array; scoring nothing"
            );
            return Vec::new();
        }
    };
    let mut scored: Vec<Scored> = Vec::new();
    let mut skipped = 0usize;
    for memory in memories {
        let Some(id) = memory["id"].as_str() else {
            skipped += 1;
            continue;
        };
        let (row, age_days, vector) = match (
            terms(&memory["terms"]),
            memory["age_days"].as_f64(),
            optional_vector(&memory["vector"]),
        ) {
            (Ok(row), Some(age_days), Ok(vector)) => (row, age_days, vector),
            _ => {
                skipped += 1;
                continue;
            }
        };
        let id = id.to_string();
        let updated_at = now - Duration::milliseconds((age_days * 86_400_000.0) as i64);

        let overlap = lexical(&query, &row);
        let recency = recency_at(updated_at, now);
        // `vector_score` returns `None` for a missing or mis-sized vector,
        // which is exactly what routes the pair to `without_vector`.
        let vector = match (query_vector, vector) {
            (Some(left), Some(right)) => vector_score(left, &right),
            _ => None,
        };
        let value = rank_score(overlap, recency, vector);
        if passes_gate(value, overlap > 0.0) {
            scored.push(Scored {
                id,
                score: value,
                updated_at,
            });
        }
    }
    if skipped > 0 {
        tracing::warn!(
            case_id,
            skipped,
            "recall fixture case skipped malformed candidates"
        );
    }

    // A wholly unrelated query yields nothing, even though one candidate
    // may have passed the individual gate above (spec §5.2 「語彙が一致した
    // 候補が 1 件も無ければ空を返す」).
    if scored.is_empty()
        || scored
            .iter()
            .map(|item| item.score)
            .fold(f64::MIN, f64::max)
            < MIN_SCORE
    {
        scored.clear();
    }
    // Same tie-break as both implementations: newer wins (spec §5.2).
    scored.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(right.updated_at.cmp(&left.updated_at))
    });
    scored.truncate(limit);
    scored
}
