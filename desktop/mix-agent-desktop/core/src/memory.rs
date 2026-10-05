//! Memory service: CRUD, revisions, restore, and conflict detection.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::retrieval::{retrieve_memories, MemoryHit};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub title: String,
    pub content: String,
    pub role: String,
    pub scope: String,
    pub source_kind: String,
    pub lifecycle_state: String,
    pub verification: String,
    pub confidence: f64,
    pub salience: f64,
    pub strength: f64,
    pub task_id: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// Valid lifecycle states, mirroring the Docker edition's widened vocabulary.
pub const LIFECYCLE_STATES: &[&str] = &[
    "candidate",
    "active",
    "established",
    "superseded",
    "expired",
    "archived",
    "disputed",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRevision {
    pub id: String,
    pub memory_id: String,
    pub title: String,
    pub content: String,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCreate {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default = "default_source")]
    pub source_kind: String,
    #[serde(default)]
    pub task_id: Option<String>,
}

fn default_source() -> String {
    "user".to_string()
}

pub struct MemoryService;

impl MemoryService {
    /// Insert a Memory item and create its initial revision.
    pub async fn create(
        storage: &mixd_storage::Storage,
        input: MemoryCreate,
    ) -> anyhow::Result<MemoryItem> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let data = serde_json::json!({
            "title": input.title,
            "content": input.content,
            "role": input.role,
            "scope": input.scope,
            "sourceKind": input.source_kind,
            "verification": "unverified",
        });
        let mut tx = storage.pool().begin().await?;
        sqlx::query(
            "INSERT INTO memories (id, owner_id, data, created_at, lifecycle_state, role, scope, \
             source_kind, confidence, salience, strength, updated_at, task_id, verification) \
             VALUES (?, ?, ?, ?, 'candidate', ?, ?, ?, 0.7, 0.5, 0.6, ?, ?, 'unverified')",
        )
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(now.to_rfc3339())
        .bind(&input.role)
        .bind(&input.scope)
        .bind(&input.source_kind)
        .bind(now.to_rfc3339())
        .bind(&input.task_id)
        .execute(&mut *tx)
        .await?;
        Self::push_revision(&mut tx, &id, &input.title, &input.content, now).await?;
        tx.commit().await?;
        Ok(MemoryItem {
            id,
            title: input.title,
            content: input.content,
            role: input.role,
            scope: input.scope,
            source_kind: input.source_kind,
            lifecycle_state: "candidate".into(),
            verification: "unverified".into(),
            confidence: 0.7,
            salience: 0.5,
            strength: 0.6,
            task_id: input.task_id,
            created_at: now,
            updated_at: now,
        })
    }

    async fn push_revision(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        memory_id: &str,
        title: &str,
        content: &str,
        now: chrono::DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO memory_revisions (id, owner_id, data, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(
            serde_json::json!({ "memoryId": memory_id, "title": title, "content": content })
                .to_string(),
        )
        .bind(now.to_rfc3339())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn update(
        storage: &mixd_storage::Storage,
        id: &str,
        title: &str,
        content: &str,
    ) -> anyhow::Result<MemoryItem> {
        let now = Utc::now();
        let mut tx = storage.pool().begin().await?;
        let data = serde_json::json!({ "title": title, "content": content });
        sqlx::query("UPDATE memories SET data = ?, updated_at = ? WHERE id = ?")
            .bind(data.to_string())
            .bind(now.to_rfc3339())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        Self::push_revision(&mut tx, id, title, content, now).await?;
        tx.commit().await?;
        Self::get(storage, id).await
    }

    pub async fn get(storage: &mixd_storage::Storage, id: &str) -> anyhow::Result<MemoryItem> {
        let row = sqlx::query(
            "SELECT id, data, verification, role, scope, source_kind, lifecycle_state, \
             confidence, salience, strength, task_id, created_at, updated_at \
             FROM memories WHERE id = ?",
        )
        .bind(id)
        .fetch_one(storage.pool())
        .await?;
        Self::from_row(&row)
    }

    fn from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<MemoryItem> {
        use sqlx::Row;
        let data: serde_json::Value = row
            .try_get::<String, _>("data")
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let ts = |key: &str| {
            row.try_get::<String, _>(key)
                .ok()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(Utc::now)
        };
        Ok(MemoryItem {
            id: row.try_get("id").unwrap_or_default(),
            title: data
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            content: data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            role: row.try_get("role").unwrap_or_else(|_| "fact".into()),
            scope: row.try_get("scope").unwrap_or_else(|_| "user".into()),
            source_kind: row.try_get("source_kind").unwrap_or_else(|_| "user".into()),
            lifecycle_state: row
                .try_get("lifecycle_state")
                .unwrap_or_else(|_| "candidate".into()),
            // The column, not the JSON blob: `verify` writes the column and
            // `update` rebuilds the blob from title/content only, so reading the
            // blob meant a verified Memory stayed `unverified` forever.
            verification: row
                .try_get("verification")
                .unwrap_or_else(|_| "unverified".into()),
            confidence: row.try_get("confidence").unwrap_or(0.7),
            salience: row.try_get("salience").unwrap_or(0.5),
            strength: row.try_get("strength").unwrap_or(0.6),
            task_id: row.try_get("task_id").ok(),
            created_at: ts("created_at"),
            updated_at: ts("updated_at"),
        })
    }

    pub async fn list(
        storage: &mixd_storage::Storage,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryItem>> {
        let rows = sqlx::query(
            "SELECT id, data, verification, role, scope, source_kind, lifecycle_state, \
             confidence, salience, strength, task_id, created_at, updated_at FROM memories \
             ORDER BY updated_at DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(storage.pool())
        .await?;
        rows.iter().map(Self::from_row).collect()
    }

    pub async fn delete(storage: &mixd_storage::Storage, id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM memories WHERE id = ?")
            .bind(id)
            .execute(storage.pool())
            .await?;
        Ok(())
    }

    pub async fn revisions(
        storage: &mixd_storage::Storage,
        memory_id: &str,
    ) -> anyhow::Result<Vec<MemoryRevision>> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT id, data, created_at FROM memory_revisions ORDER BY created_at DESC",
        )
        .fetch_all(storage.pool())
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let data: serde_json::Value = row
                    .try_get::<String, _>("data")
                    .ok()
                    .and_then(|raw| serde_json::from_str(&raw).ok())?;
                if data.get("memoryId")?.as_str()? != memory_id {
                    return None;
                }
                Some(MemoryRevision {
                    id: row.try_get("id").unwrap_or_default(),
                    memory_id: memory_id.to_string(),
                    title: data.get("title")?.as_str()?.to_string(),
                    content: data.get("content")?.as_str()?.to_string(),
                    created_at: row
                        .try_get::<String, _>("created_at")
                        .ok()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|t| t.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now),
                })
            })
            .collect())
    }

    /// Restore a Memory item to the state captured by a revision.
    pub async fn restore(
        storage: &mixd_storage::Storage,
        memory_id: &str,
        revision_id: &str,
    ) -> anyhow::Result<MemoryItem> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT data FROM memory_revisions WHERE id = ?")
                .bind(revision_id)
                .fetch_optional(storage.pool())
                .await?;
        let data: serde_json::Value = row
            .and_then(|(raw,)| serde_json::from_str(&raw).ok())
            .ok_or_else(|| anyhow::anyhow!("revision not found"))?;
        // The revision must belong to this memory. Without this check any
        // revision id could overwrite any memory with another memory's content.
        if data.get("memoryId").and_then(|v| v.as_str()) != Some(memory_id) {
            anyhow::bail!("revision does not belong to this memory");
        }
        let title = data
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let content = data
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        Self::update(storage, memory_id, title, content).await
    }

    /// Promote a candidate to `active` once it has been confirmed.
    pub async fn verify(
        storage: &mixd_storage::Storage,
        id: &str,
        verdict: &str,
    ) -> anyhow::Result<()> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE memories SET lifecycle_state = 'active', verification = ?, updated_at = ? WHERE id = ?",
        )
        .bind(verdict)
        .bind(now.to_rfc3339())
        .bind(id)
        .execute(storage.pool())
        .await?;
        Ok(())
    }

    /// Bump salience / strength after the agent re-reads an item.
    pub async fn reinforce(storage: &mixd_storage::Storage, id: &str) -> anyhow::Result<()> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE memories SET activation_count = activation_count + 1, \
             last_activated_at = ?, last_reinforced_at = ?, \
             salience = MIN(1.0, salience + 0.05), \
             strength = MIN(1.0, strength + 0.03), updated_at = ? WHERE id = ?",
        )
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(id)
        .execute(storage.pool())
        .await?;
        Ok(())
    }

    pub async fn search(
        storage: &mixd_storage::Storage,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryHit>> {
        retrieve_memories(storage, query, limit).await
    }
}
