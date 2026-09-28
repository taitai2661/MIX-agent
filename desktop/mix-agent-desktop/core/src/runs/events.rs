//! Run event envelope shared with the Tauri command surface. Field shapes
//! match the Docker edition's `GET /runs/{key}/events` contract.

use serde::{Deserialize, Serialize};

use crate::runs::state::RunId;

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
