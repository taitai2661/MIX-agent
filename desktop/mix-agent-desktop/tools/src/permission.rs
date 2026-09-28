//! Permission policy engine.
//!
//! Phase 2 implements the Always Allow / Ask / Deny state machine and the
//! persistent approval records.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    #[default]
    Ask,
    Deny,
}
