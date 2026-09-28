//! Checkpoint storage: keeps the most recent N snapshots per Run so the user
//! can resume from an exact step instead of starting over.

use mixd_storage::Storage;

use super::engine::CheckpointSnapshot;

pub struct CheckpointStore {
    pub keep: usize,
}

impl CheckpointStore {
    pub fn new(keep: usize) -> Self {
        Self { keep }
    }

    /// Persist a snapshot and return its id.
    pub async fn save(
        &self,
        run_id: uuid::Uuid,
        step: u32,
        tool_count: u32,
        snapshot: &CheckpointSnapshot,
        storage: &Storage,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let data = serde_json::json!({
            "history": snapshot.messages,
            "counters": { "step": step, "toolCount": tool_count },
            "trigger": "interval",
        });
        let insert = sqlx::query(
            "INSERT INTO run_checkpoints (id, owner_id, data, run_id, step, tool_count, trigger) \
             VALUES (?, ?, ?, ?, ?, ?, 'interval')",
        )
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(run_id.to_string())
        .bind(step as i64)
        .bind(tool_count as i64)
        .execute(storage.pool())
        .await;
        if let Err(err) = &insert {
            // The `run_checkpoints.run_id` foreign key makes this fail whenever
            // the parent run row is missing, and the result was discarded — so a
            // run could silently have no resumable state and nothing would say
            // why.
            tracing::warn!(?err, %run_id, step, "checkpoint insert failed");
        }

        // Trim old snapshots.
        let _ = sqlx::query(
            "DELETE FROM run_checkpoints WHERE run_id = ? AND id NOT IN (\
               SELECT id FROM run_checkpoints WHERE run_id = ? ORDER BY step DESC, created_at DESC LIMIT ?\
             )",
        )
        .bind(run_id.to_string())
        .bind(run_id.to_string())
        .bind(self.keep as i64)
        .execute(storage.pool())
        .await;

        id
    }

    pub async fn load(
        &self,
        run_id: uuid::Uuid,
        checkpoint_id: &str,
        storage: &Storage,
    ) -> Option<CheckpointSnapshot> {
        let row: Option<(String, i64)> = sqlx::query_as::<_, (String, i64)>(
            "SELECT data, step FROM run_checkpoints WHERE id = ? AND run_id = ?",
        )
        .bind(checkpoint_id)
        .bind(run_id.to_string())
        .fetch_optional(storage.pool())
        .await
        .unwrap_or(None);

        let (raw, step) = row?;
        let data: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let history = data
            .get("history")
            .cloned()
            .unwrap_or(serde_json::json!([]));
        let messages: Vec<super::engine::HistoryMessage> =
            serde_json::from_value(history).unwrap_or_default();
        Some(CheckpointSnapshot {
            step_index: step as u32,
            messages,
        })
    }

    /// List checkpoint metadata for the UI, newest first.
    pub async fn list(&self, run_id: uuid::Uuid, storage: &Storage) -> Vec<(String, u32, String)> {
        sqlx::query_as::<_, (String, i64, String)>(
            "SELECT id, step, created_at FROM run_checkpoints WHERE run_id = ? \
             ORDER BY step DESC, created_at DESC",
        )
        .bind(run_id.to_string())
        .fetch_all(storage.pool())
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|(id, step, created)| (id, step as u32, created))
                .collect()
        })
        .unwrap_or_default()
    }
}
