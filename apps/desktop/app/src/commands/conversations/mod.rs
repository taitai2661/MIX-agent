use std::sync::Arc;

use crate::spotlight::payload::SpotlightKind;
use crate::AppState;
use base64::Engine as _;
use chrono::Utc;
use mixd_core::runs::engine::{AutoFallback, HistoryMessage, RunRequest};
use mixd_core::runs::state::{RunId, RunStatus};
use mixd_providers::{ReasoningControl, ToolSpec};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

/// Query/shape constants for this module. The numbers used to be inline
/// literals scattered across SQL strings and call sites, so changing the
/// sidebar preview length meant finding every `140` by hand.
pub mod limits {
    /// Sidebar preview excerpt length (`substr(content, 1, N)`).
    pub const PREVIEW_CHARS: i64 = 140;
    /// Transcript turns handed to the engine (`LIMIT` in `load_history`).
    pub const HISTORY_LIMIT: i64 = 80;
    /// Prior-turn context parts handed to Auto routing (`LIMIT 20`).
    pub const AUTO_CONTEXT_LIMIT: i64 = 20;
    /// Default `list_messages` page size. The transcript loads older pages on
    /// demand instead of dumping every row in one IPC round trip.
    pub const DEFAULT_MESSAGE_LIMIT: u32 = 100;
    /// Hard ceiling for `list_messages` pages.
    pub const MAX_MESSAGE_LIMIT: u32 = 500;
    /// Default and maximum page size for the paginated thread list. The
    /// sidebar fetches these pages on demand; 50 keeps the first paint small
    /// on an installation with years of history.
    pub const DEFAULT_CONVERSATION_LIMIT: u32 = 50;
    pub const MAX_CONVERSATION_LIMIT: u32 = 200;
    /// Largest accepted user message (chars) and attachment payloads.
    pub const MAX_CONTENT_CHARS: usize = 200_000;
    pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
    pub const MAX_ATTACHMENTS_TOTAL_BYTES: usize = 20 * 1024 * 1024;
    pub const MAX_ATTACHMENTS: usize = 10;
    /// Output tokens reserved when Auto estimates the required context.
    pub const AUTO_RESERVED_OUTPUT_TOKENS: u64 = 4096;

    /// Per-mode output reservation. Agent turns emit tool calls plus prose, and
    /// thinking turns can spend a large reasoning budget; chat is smallest.
    pub fn auto_reserved_output_tokens(mode: mixd_core::Mode) -> u64 {
        match mode {
            mixd_core::Mode::Chat => 2048,
            mixd_core::Mode::Thinking => 8192,
            mixd_core::Mode::Plan => 4096,
            mixd_core::Mode::Agent => 8192,
        }
    }

    /// Stable `request_key` for idempotency: `conv:<id>:<millis>:<run>`.
    pub fn request_key(conversation_id: &str, now_millis: i64, run_id: &str) -> String {
        format!("conv:{conversation_id}:{now_millis}:{run_id}")
    }
}

mod crud;
mod messages;
mod run;
mod summary;
mod support;
mod views;

pub use crud::*;
pub use messages::*;
pub use run::*;
pub use summary::*;
pub use support::*;
pub use views::*;
