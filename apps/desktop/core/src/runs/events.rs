//! Run event envelope shared with the Tauri command surface. Field shapes
//! match the Docker edition's `GET /runs/{key}/events` contract.

use serde::{Deserialize, Serialize};

use crate::runs::state::RunId;

/// Upper bound on rows one replay will read.
///
/// Stream deltas are coalesced at write time, so a real run is far below this;
/// the bound exists so a legacy or pathological run cannot make opening a
/// transcript allocate an unbounded vector. Structural events sit at the head
/// of the log, so keeping the first N preserves the tool timeline.
pub const MAX_REPLAY_EVENTS: i64 = 50_000;

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
