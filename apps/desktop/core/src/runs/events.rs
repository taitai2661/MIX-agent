//! Run event envelope shared with the Tauri command surface. Field shapes
//! match the Docker edition's `GET /runs/{key}/events` contract.

use serde::{Deserialize, Serialize};

use crate::runs::state::RunId;

/// Upper bound on rows one replay will read.
///
/// Stream deltas are coalesced at write time, so a real run is far below this;
/// the bound exists so a legacy or pathological run cannot make opening a
/// transcript allocate an unbounded vector. A truncated replay keeps both the
/// head and the tail: the head preserves the tool timeline, and the tail keeps
/// the latest answer and the terminal status. A `replay_truncated` marker
/// records how many rows were omitted.
pub const MAX_REPLAY_EVENTS: i64 = 50_000;

/// One persisted event row in the shape shared by both replay paths.
pub type ReplayRow = (String, i64, String, String, String);

/// Rows returned by [`load_replay_window`] plus the untruncated row count.
#[derive(Debug, Default)]
pub struct ReplayWindow {
    pub rows: Vec<ReplayRow>,
    pub total: i64,
}

impl ReplayWindow {
    /// Number of events the replay window omitted.
    pub fn omitted(&self, limit: i64) -> i64 {
        (self.total - limit).max(0)
    }
}

/// Load at most `limit` events, keeping the oldest and newest halves.
///
/// `COUNT(*)` uses the `(run_id, sequence)` index, so a short run costs one
/// count plus one ordered read. A long run reads `limit / 2` rows from each
/// end instead of loading the whole (potentially huge) log and discarding the
/// part the user most wants to see.
pub async fn load_replay_window(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    run_id: &str,
    limit: i64,
) -> Result<ReplayWindow, sqlx::Error> {
    if limit <= 0 {
        return Ok(ReplayWindow::default());
    }
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_events WHERE run_id = ?")
        .bind(run_id)
        .fetch_one(pool)
        .await?;
    if total <= limit {
        let rows: Vec<ReplayRow> = sqlx::query_as(
            "SELECT run_id, sequence, kind, data, created_at FROM run_events \
             WHERE run_id = ? ORDER BY sequence ASC LIMIT ?",
        )
        .bind(run_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        return Ok(ReplayWindow { rows, total });
    }

    // Give the tail at least one slot even for a tiny limit, because the
    // newest status/answer is the part a truncated replay must not lose.
    let tail_limit = (limit / 2).max(1);
    let head_limit = limit - tail_limit;
    let mut rows: Vec<ReplayRow> = if head_limit > 0 {
        sqlx::query_as(
            "SELECT run_id, sequence, kind, data, created_at FROM run_events \
             WHERE run_id = ? ORDER BY sequence ASC LIMIT ?",
        )
        .bind(run_id)
        .bind(head_limit)
        .fetch_all(pool)
        .await?
    } else {
        Vec::new()
    };
    let mut tail: Vec<ReplayRow> = sqlx::query_as(
        "SELECT run_id, sequence, kind, data, created_at FROM run_events \
         WHERE run_id = ? ORDER BY sequence DESC LIMIT ?",
    )
    .bind(run_id)
    .bind(tail_limit)
    .fetch_all(pool)
    .await?;
    tail.reverse();
    rows.extend(tail);
    Ok(ReplayWindow { rows, total })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub run_id: RunId,
    pub sequence: i64,
    pub kind: String,
    pub data: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Convenience trait so the storage layer can also surface Run events
/// without depending on the broadcast channel directly.
#[async_trait::async_trait]
pub trait RunEventSink: Send + Sync {
    async fn emit(&self, event: RunEvent);
}
