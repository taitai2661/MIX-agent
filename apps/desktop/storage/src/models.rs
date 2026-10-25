//! Domain model types — re-exports for ergonomic imports.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub id: String,
    pub run_id: String,
    pub sequence: i64,
    pub kind: String,
    pub data: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub run_id: String,
    pub status: String,
    pub tool_name: String,
    pub arguments: Value,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub approval_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub run_id: String,
    pub tool_call_id: String,
    pub status: String,
    pub tool_name: String,
    pub arguments: Value,
    pub granted_at: Option<DateTime<Utc>>,
    pub remember: bool,
    pub expires: DateTime<Utc>,
}
