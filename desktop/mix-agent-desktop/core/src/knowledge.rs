//! Knowledge base: chunked text documents with FTS-style search.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeDoc {
    pub doc_id: String,
    pub filename: String,
    pub chunk_count: usize,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeChunk {
    pub id: String,
    pub doc_id: String,
    pub filename: String,
    pub chunk_index: u32,
    pub text: String,
    pub created_at: chrono::DateTime<Utc>,
}

/// Target chunk size in characters. Chunks are split on paragraph boundaries
/// where possible so a retrieved chunk stays readable.
pub const CHUNK_CHARS: usize = 1200;

pub struct KnowledgeService;

impl KnowledgeService {
    /// Ingest a document, splitting it into chunks.
    pub async fn ingest(
        storage: &mixd_storage::Storage,
        filename: &str,
        text: &str,
    ) -> anyhow::Result<KnowledgeDoc> {
        let doc_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let chunks = split_into_chunks(text, CHUNK_CHARS);
        let mut tx = storage.pool().begin().await?;
        for (index, chunk) in chunks.iter().enumerate() {
            sqlx::query(
                "INSERT INTO knowledge (id, owner_id, data, created_at) VALUES (?, ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(mixd_storage::records::SINGLE_USER_ID)
            .bind(
                serde_json::json!({
                    "docId": doc_id,
                    "filename": filename,
                    "chunkIndex": index,
                    "text": chunk,
                })
                .to_string(),
            )
            .bind(now.to_rfc3339())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(KnowledgeDoc {
            doc_id,
            filename: filename.to_string(),
            chunk_count: chunks.len(),
            created_at: now,
        })
    }

    pub async fn list(storage: &mixd_storage::Storage) -> anyhow::Result<Vec<KnowledgeDoc>> {
        use sqlx::Row;
        let rows = sqlx::query("SELECT data, created_at FROM knowledge ORDER BY created_at DESC")
            .fetch_all(storage.pool())
            .await?;
        let mut docs: Vec<KnowledgeDoc> = Vec::new();
        for row in rows {
            let data: serde_json::Value = row
                .try_get::<String, _>("data")
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default();
            let doc_id = data
                .get("docId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let filename = data
                .get("filename")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let created = row
                .try_get::<String, _>("created_at")
                .ok()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);
            match docs.iter_mut().find(|d| d.doc_id == doc_id) {
                Some(existing) => existing.chunk_count += 1,
                None => docs.push(KnowledgeDoc {
                    doc_id,
                    filename,
                    chunk_count: 1,
                    created_at: created,
                }),
            }
        }
        Ok(docs)
    }

    pub async fn delete_doc(
        storage: &mixd_storage::Storage,
        doc_id: &str,
    ) -> anyhow::Result<usize> {
        use sqlx::Row;
        let rows = sqlx::query("SELECT id, data FROM knowledge")
            .fetch_all(storage.pool())
            .await?;
        let mut ids = Vec::new();
        for row in rows {
            let data: serde_json::Value = row
                .try_get::<String, _>("data")
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default();
            if data.get("docId").and_then(|v| v.as_str()) == Some(doc_id) {
                ids.push(row.try_get::<String, _>("id").unwrap_or_default());
            }
        }
        for id in &ids {
            sqlx::query("DELETE FROM knowledge WHERE id = ?")
                .bind(id)
                .execute(storage.pool())
                .await?;
        }
        Ok(ids.len())
    }
}

/// Split text into chunks on paragraph boundaries, falling back to a hard cut.
pub fn split_into_chunks(text: &str, target: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for paragraph in text.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        if current.chars().count() + paragraph.chars().count() + 2 <= target {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(paragraph);
            continue;
        }
        if !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if paragraph.chars().count() <= target {
            current = paragraph.to_string();
            continue;
        }
        // Oversized paragraph: prefer line boundaries, then fall back to a
        // hard character cut so a chunk never exceeds the target.
        let mut buf = String::new();
        for line in paragraph.lines() {
            if buf.chars().count() + line.chars().count() + 1 > target {
                chunks.push(std::mem::take(&mut buf));
            }
            if line.chars().count() > target {
                // This single line is longer than a whole chunk.
                let mut rest = line;
                while rest.chars().count() > target {
                    // `target` is a character budget but `split_at` counts
                    // bytes: slicing a Japanese line at `target` landed in the
                    // middle of a character and panicked — and the release
                    // profile is `panic = "abort"`, so it killed the app.
                    let mut cut = target.min(rest.len());
                    while cut > 0 && !rest.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    if cut == 0 {
                        cut = 1;
                        while cut < rest.len() && !rest.is_char_boundary(cut) {
                            cut += 1;
                        }
                    }
                    if cut == 0 || cut >= rest.len() {
                        chunks.push(rest.to_string());
                        rest = "";
                        break;
                    }
                    let (head, tail) = rest.split_at(cut);
                    chunks.push(head.to_string());
                    rest = tail;
                }
                buf.push_str(rest);
                continue;
            }
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(line);
        }
        current = buf;
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_paragraphs() {
        let text = "aaaa\n\nbbbb\n\ncccc\n\ndddd\n\neeee\n\nffff\n\ngggg\n\nhhhh";
        let chunks = split_into_chunks(text, 12);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.chars().count() <= 12, "chunk too long: {:?}", c);
        }
    }

    #[test]
    fn hard_splits_oversized_paragraph() {
        let text = "x".repeat(300);
        let chunks = split_into_chunks(&text, 100);
        assert!(chunks.len() >= 3);
        for c in &chunks {
            assert!(c.chars().count() <= 100, "chunk too long: {}", c.len());
        }
    }

    #[test]
    fn a_multibyte_line_is_split_on_a_character_boundary() {
        // One long line with no spaces, exactly what Japanese prose looks like.
        let text = "あ".repeat(500);
        let chunks = split_into_chunks(&text, 100);
        assert!(chunks.len() >= 5);
        assert_eq!(chunks.concat(), text);
        for c in &chunks {
            assert!(c.chars().count() <= 100, "chunk too long: {:?}", c);
        }
    }

    #[test]
    fn a_chunk_that_starts_with_a_multibyte_character_is_reassembled() {
        // Byte index `target` inside the very first character.
        let text = format!("{}{}", "あ".repeat(3), "b".repeat(50));
        let chunks = split_into_chunks(&text, 1);
        assert_eq!(chunks.concat(), text);
    }
}
