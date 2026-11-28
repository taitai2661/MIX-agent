//! Read-only tool-call history for a conversation.
//!
//! The transcript already renders live tool activity, but after a restart the
//! historical calls were only reachable through `run_events`. This exposes the
//! durable `tool_calls` rows (the same data the audit trail is built from) so
//! the UI can show what a past run actually executed, without replaying every
//! event.

use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ToolCallHistoryView {
    pub id: String,
    pub run_id: String,
    pub status: String,
    pub tool_name: String,
    /// Redacted before it leaves Rust: `cat .env` must not put a live key into
    /// a history panel.
    pub arguments: serde_json::Value,
    pub result: Option<serde_json::Value>,
    /// Tool errors can embed paths and tokens, so they are redacted too.
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub approval_id: Option<String>,
}

/// A page of a conversation's tool calls, oldest first.
///
/// The query takes the *newest* rows so a long conversation does not silently
/// lose its most recent activity, then reverses them for display. `truncated`
/// tells the caller that older calls exist but were not returned.
#[derive(Serialize)]
pub struct ToolCallHistoryPage {
    pub calls: Vec<ToolCallHistoryView>,
    pub truncated: bool,
}

/// The most recent 2000 tool calls made in a conversation, oldest first.
#[tauri::command]
pub async fn list_conversation_tool_calls(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<ToolCallHistoryPage, String> {
    use sqlx::Row as _;

    const LIMIT: usize = 2000;
    // Read one extra row so the caller can be told that older calls exist.
    let rows = sqlx::query(
        "SELECT tc.id, tc.run_id, tc.status, tc.data \
         FROM tool_calls tc JOIN runs r ON r.id = tc.run_id \
         WHERE r.conversation_id = ? ORDER BY tc.created_at DESC, tc.id DESC \
         LIMIT ?",
    )
    .bind(&conversation_id)
    .bind(LIMIT as i64 + 1)
    .fetch_all(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    let truncated = rows.len() > LIMIT;
    let rows = &rows[..rows.len().min(LIMIT)];

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let raw: String = row.try_get("data").map_err(crate::error::cmd_err)?;
        let Ok(data) = serde_json::from_str::<mixd_storage::records::ToolCallData>(&raw) else {
            // A row written by an older build is skipped rather than failing
            // the whole history; one malformed record must not hide the rest.
            continue;
        };
        let mut arguments = data.arguments;
        mixd_tools::redact::redact_value(&mut arguments);
        let mut result = if data.result.is_null() {
            None
        } else {
            Some(data.result)
        };
        if let Some(value) = result.as_mut() {
            mixd_tools::redact::redact_value(value);
        }
        out.push(ToolCallHistoryView {
            id: row.try_get("id").map_err(crate::error::cmd_err)?,
            run_id: row.try_get("run_id").map_err(crate::error::cmd_err)?,
            status: row.try_get("status").map_err(crate::error::cmd_err)?,
            tool_name: data.tool_name,
            arguments,
            result,
            error: data.error.as_deref().map(mixd_tools::redact::redact_text),
            started_at: data.started_at.to_rfc3339(),
            finished_at: data.finished_at.map(|t| t.to_rfc3339()),
            approval_id: data.approval_id,
        });
    }
    out.reverse();
    Ok(ToolCallHistoryPage {
        calls: out,
        truncated,
    })
}
