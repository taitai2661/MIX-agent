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
    /// Run the Tool once.
    Approve {
        /// Persist the rule to disk ("Always Allow").
        remember: bool,
        /// Allow the same tool/target until the app exits. Neither a one-off
        /// approval nor a durable rule: useful for a run that needs the same
        /// command several times without granting it forever.
        remember_session: bool,
    },
    Deny,
}
