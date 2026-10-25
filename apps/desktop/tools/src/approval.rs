//! Approval inbox: serialises a Tool call's pending permission decision so the
//! UI can present an Allow / Deny / Always Allow prompt.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub approval_id: String,
    pub run_id: uuid::Uuid,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub remember: bool,
}

/// The user's answer to a pending approval prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// Run the Tool once. `remember` mirrors "Always Allow".
    Approve {
        remember: bool,
    },
    Deny,
}
