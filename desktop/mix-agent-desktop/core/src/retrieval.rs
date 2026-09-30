//! Retrieval: pulls relevant Memory items and Knowledge chunks into a Run's
//! prompt. This is the Desktop counterpart of the Docker edition's
//! `mix_agent/context/retrievers.py`.

use chrono::Utc;
use mixd_storage::Storage;
use serde::{Deserialize, Serialize};

use crate::providers_bridge::Message;

/// How many Memory items to inject by default.
pub const DEFAULT_MEMORY_LIMIT: usize = 8;
/// How many Knowledge chunks to inject by default.
pub const DEFAULT_KNOWLEDGE_LIMIT: usize = 6;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetrievedBlock {
    pub memories: Vec<MemoryHit>,
    pub knowledge: Vec<KnowledgeHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHit {
    pub id: String,
    pub title: String,
    pub content: String,
    pub role: String,
    pub scope: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeHit {
    pub id: String,
    pub doc_id: String,
    pub filename: String,
    pub text: String,
    pub score: f64,
}

impl RetrievedBlock {
    pub fn is_empty(&self) -> bool {
        self.memories.is_empty() && self.knowledge.is_empty()
    }

    /// Render the block as a single `user`-role message the model can read.
    pub fn to_message(&self) -> Option<Message> {
        if self.is_empty() {
            return None;
        }
        let mut out = String::from("<retrieved_context>\n");
        if !self.memories.is_empty() {
            out.push_str("<memories>\n");
            for m in &self.memories {
                out.push_str(&format!("- [{}] {}: {}\n", m.role, m.title, m.content));
            }
            out.push_str("</memories>\n");
        }
        if !self.knowledge.is_empty() {
            out.push_str("<knowledge>\n");
            for k in &self.knowledge {
                out.push_str(&format!("- {} ({}): {}\n", k.filename, k.doc_id, k.text));
            }
            out.push_str("</knowledge>\n");
        }
        out.push_str("</retrieved_context>");
        Some(Message::text("user", out))
    }
}

/// The query, tokenised once.
///
/// Scoring walks every row of both tables, and the token list depends only on
/// the query, so it is split, lowercased and allocated a single time instead of
/// once per row scanned.
struct QueryTerms(Vec<String>);

impl QueryTerms {
    fn new(query: &str) -> Self {
        Self(
            query
                .to_lowercase()
                .split_whitespace()
                .filter(|t| t.len() > 1)
                .map(|t| t.to_string())
                .collect(),
        )
    }

    /// The fraction of the query terms present in `lowered`.
    fn hits(&self, lowered: &str) -> f64 {
        if self.0.is_empty() {
            return 0.0;
        }
        let found = self
            .0
            .iter()
            .filter(|t| lowered.contains(t.as_str()))
            .count();
        found as f64 / self.0.len() as f64
    }

    /// Score against a single haystack with a cheap token-overlap heuristic.
    /// This is intentionally simple and deterministic; a vector index lands
    /// later.
    fn score(&self, haystack: &str) -> f64 {
        self.hits(&haystack.to_lowercase())
    }

    /// Score against two fields without concatenating them per row.
    ///
    /// Query terms come from `split_whitespace`, so none of them contains a
    /// space; a term can therefore never straddle the join, and this is exactly
    /// what scoring `"title content"` produced — minus one `String` the size of
    /// a Memory item for every row scanned.
    fn score_parts(&self, a: &str, b: &str) -> f64 {
        if self.0.is_empty() {
            return 0.0;
        }
        let (a, b) = (a.to_lowercase(), b.to_lowercase());
        let found = self
            .0
            .iter()
            .filter(|t| a.contains(t.as_str()) || b.contains(t.as_str()))
            .count();
        found as f64 / self.0.len() as f64
    }
}

/// Retrieve Memory items relevant to `query`.
pub async fn retrieve_memories(
    storage: &Storage,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<MemoryHit>> {
    retrieve_memories_with(storage, &QueryTerms::new(query), limit).await
}

async fn retrieve_memories_with(
    storage: &Storage,
    terms: &QueryTerms,
    limit: usize,
) -> anyhow::Result<Vec<MemoryHit>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id, data, role, scope, salience, strength, confidence \
         FROM memories WHERE lifecycle_state NOT IN ('archived', 'expired')",
    )
    .fetch_all(storage.pool())
    .await?;

    let mut scored: Vec<MemoryHit> = rows
        .into_iter()
        .filter_map(|row| {
            let data: serde_json::Value = row
                .try_get::<String, _>("data")
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())?;
            let title = data.get("title")?.as_str()?.to_string();
            let content = data.get("content")?.as_str()?.to_string();
            let relevance = terms.score_parts(&title, &content);
            let salience: f64 = row.try_get("salience").unwrap_or(0.5);
            let strength: f64 = row.try_get("strength").unwrap_or(0.6);
            let confidence: f64 = row.try_get("confidence").unwrap_or(0.7);
            let combined = relevance * 0.6 + salience * 0.15 + strength * 0.15 + confidence * 0.1;
            if combined <= 0.0 {
                return None;
            }
            Some(MemoryHit {
                id: row.try_get::<String, _>("id").unwrap_or_default(),
                title,
                content,
                role: row
                    .try_get::<String, _>("role")
                    .unwrap_or_else(|_| "fact".into()),
                scope: row
                    .try_get::<String, _>("scope")
                    .unwrap_or_else(|_| "user".into()),
                score: combined,
            })
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit);
    Ok(scored)
}

/// Retrieve Knowledge chunks relevant to `query`.
pub async fn retrieve_knowledge(
    storage: &Storage,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<KnowledgeHit>> {
    retrieve_knowledge_with(storage, &QueryTerms::new(query), limit).await
}

async fn retrieve_knowledge_with(
    storage: &Storage,
    terms: &QueryTerms,
    limit: usize,
) -> anyhow::Result<Vec<KnowledgeHit>> {
    use sqlx::Row;
    let rows = sqlx::query("SELECT id, data, created_at FROM knowledge")
        .fetch_all(storage.pool())
        .await?;
    let now = Utc::now();
    let _ = now;
    let mut scored: Vec<KnowledgeHit> = rows
        .into_iter()
        .filter_map(|row| {
            let data: serde_json::Value = row
                .try_get::<String, _>("data")
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())?;
            let text = data.get("text")?.as_str()?.to_string();
            let filename = data
                .get("filename")
                .and_then(|v| v.as_str())
                .unwrap_or("document")
                .to_string();
            let s = terms.score(&text);
            if s <= 0.0 {
                return None;
            }
            Some(KnowledgeHit {
                id: row.try_get::<String, _>("id").unwrap_or_default(),
                doc_id: data
                    .get("docId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                filename,
                text,
                score: s,
            })
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit);
    Ok(scored)
}

/// Convenience: retrieve both Memory and Knowledge in one call.
///
/// The two lookups are independent tables, so they run concurrently rather than
/// one after the other.
pub async fn retrieve_all(storage: &Storage, query: &str) -> anyhow::Result<RetrievedBlock> {
    let terms = QueryTerms::new(query);
    let (memories, knowledge) = tokio::try_join!(
        retrieve_memories_with(storage, &terms, DEFAULT_MEMORY_LIMIT),
        retrieve_knowledge_with(storage, &terms, DEFAULT_KNOWLEDGE_LIMIT),
    )?;
    Ok(RetrievedBlock {
        memories,
        knowledge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_is_normalised() {
        let terms = QueryTerms::new("alpha beta");
        assert_eq!(terms.score("alpha beta gamma"), 1.0);
        assert_eq!(terms.score("gamma delta"), 0.0);
        assert_eq!(QueryTerms::new("").score("anything"), 0.0);
    }

    /// Scoring two fields must be indistinguishable from scoring their
    /// concatenation: a term is whitespace-split, so it can never straddle the
    /// join the concatenation would insert.
    #[test]
    fn score_parts_matches_the_concatenated_haystack() {
        let terms = QueryTerms::new("prefer rust");
        for (a, b) in [
            ("Prefers Rust", "over Go"),
            ("prefers go", ""),
            ("", "I prefer rust a lot"),
        ] {
            let joined = format!("{a} {b}");
            assert_eq!(
                terms.score_parts(a, b),
                terms.score(&joined),
                "mismatch for ({a:?}, {b:?})"
            );
        }
    }

    #[test]
    fn block_renders_and_can_be_empty() {
        let empty = RetrievedBlock::default();
        assert!(empty.to_message().is_none());

        let block = RetrievedBlock {
            memories: vec![MemoryHit {
                id: "1".into(),
                title: "Prefers Rust".into(),
                content: "The user prefers Rust over Go.".into(),
                role: "user".into(),
                scope: "user".into(),
                score: 0.9,
            }],
            knowledge: Vec::new(),
        };
        let msg = block.to_message().expect("message");
        assert!(msg.content.contains("Prefers Rust"));
        assert!(msg.content.contains("<retrieved_context>"));
    }
}
