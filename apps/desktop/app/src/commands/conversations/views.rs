use super::*;

/// One persisted message of a conversation, in the shape the transcript view
/// renders. `reasoning` is written into the JSON payload by `dispatch_run` but
/// is not part of `MessageData`, so the raw JSON is read instead of the typed
/// record.
#[derive(Debug, Clone, Serialize)]
pub struct MessageView {
    pub id: String,
    pub role: String,
    pub content: String,
    pub reasoning: Option<String>,
    pub model_id: Option<String>,
    pub run_id: Option<String>,
    pub kind: String,
    pub created_at: String,
    pub attachments: Vec<MessageAttachmentView>,
    /// SQLite rowid. Stable across edits and used as the "older than" cursor.
    pub seq: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageAttachmentView {
    pub kind: String,
    pub mime_type: String,
    pub filename: Option<String>,
}

#[tauri::command]
pub async fn list_messages(
    state: State<'_, AppState>,
    conversation_id: String,
    limit: Option<u32>,
    before: Option<i64>,
) -> Result<Vec<MessageView>, String> {
    // Clamp the page size: an unbounded LIMIT lets a caller dump the whole
    // table in one IPC round trip.
    let limit = limit
        .unwrap_or(limits::DEFAULT_MESSAGE_LIMIT)
        .clamp(1, limits::MAX_MESSAGE_LIMIT);
    fetch_messages_before(state.storage.pool(), &conversation_id, limit, before).await
}

/// Query helper kept separate from the command for integration tests.
///
/// Only `user` / `assistant` rows are transcript. `tool` rows (raw JSON
/// results) are persisted for the model's next turn but must never render
/// as chat bubbles — they used to appear as assistant messages showing raw
/// `{"args":...}` / `{"body":"<!doctype..."}` payloads.
pub async fn fetch_messages(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    conversation_id: &str,
    limit: u32,
) -> Result<Vec<MessageView>, String> {
    fetch_messages_before(pool, conversation_id, limit, None).await
}

/// Fetch one page of transcript rows strictly older than `before` (or the
/// newest page when `before` is `None`). `rowid` is the cursor: it is stable,
/// monotonic, and does not collide when two messages share a timestamp.
pub async fn fetch_messages_before(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    conversation_id: &str,
    limit: u32,
    before: Option<i64>,
) -> Result<Vec<MessageView>, String> {
    let limit = limit.clamp(1, limits::MAX_MESSAGE_LIMIT);
    let rows: Vec<MessageRow> = sqlx::query_as(
        "SELECT rowid, id, role, content, \
                json_extract(data, '$.reasoning') AS reasoning, \
                json_extract(data, '$.modelId') AS model_id, \
                json_extract(data, '$.runId') AS run_id, \
                json_extract(data, '$.kind') AS kind, \
                created_at, \
                json_extract(data, '$.attachments') AS attachments \
         FROM messages WHERE conversation_id = ? \
           AND role IN ('user', 'assistant') \
           AND (? IS NULL OR rowid < ?) \
         ORDER BY rowid DESC LIMIT ?",
    )
    .bind(conversation_id)
    .bind(before)
    .bind(before)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        // Newest-first fetch, oldest-first render — and past the limit the
        // visible end of the transcript is the recent one, not message #1.
        .rev()
        .map(
            |(
                seq,
                id,
                role,
                content,
                reasoning,
                model_id,
                run_id,
                kind,
                created_at,
                attachments,
            )| {
                MessageView {
                    id,
                    role: role.unwrap_or_default(),
                    content: content.unwrap_or_default(),
                    reasoning,
                    model_id,
                    run_id,
                    kind: kind.unwrap_or_default(),
                    created_at: normalize_ts(&created_at),
                    attachments: parse_attachments(attachments.as_deref()),
                    seq,
                }
            },
        )
        .collect())
}

pub type MessageRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
);

pub fn normalize_ts(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Utc).to_rfc3339())
        .unwrap_or_else(|_| value.to_string())
}

pub fn parse_attachments(raw: Option<&str>) -> Vec<MessageAttachmentView> {
    let raw = match raw {
        Some(raw) if !raw.is_empty() => raw,
        _ => return Vec::new(),
    };
    // Reuse the storage record's serde renames (`mimeType`, ...) for parsing.
    let parsed: Vec<mixd_storage::records::MessageAttachment> =
        serde_json::from_str(raw).unwrap_or_default();
    parsed
        .into_iter()
        .map(|a| MessageAttachmentView {
            kind: a.kind,
            mime_type: a.mime_type,
            filename: a.filename,
        })
        .collect()
}

#[tauri::command]
pub async fn export_markdown(
    state: State<'_, AppState>,
    id: String,
    include_tool_calls: bool,
) -> Result<String, String> {
    use sqlx::Row;
    let pool = state.storage.pool();
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT json_extract(data, '$.title') FROM conversations WHERE id = ?")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .map_err(crate::error::cmd_err)?;
    let Some((title,)) = row else {
        return Err(format!("conversation not found: {id}"));
    };
    let rows = sqlx::query(
        "SELECT json_extract(data, '$.role') AS role, json_extract(data, '$.content') AS content \
         FROM messages WHERE conversation_id = ? ORDER BY created_at ASC, rowid ASC",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;

    let mut out = String::new();
    out.push_str(&format!(
        "# {}\n\n",
        title.unwrap_or_else(|| "MIX-agent".into())
    ));
    for row in rows {
        let role: String = row
            .try_get::<Option<String>, _>("role")
            .ok()
            .flatten()
            .unwrap_or_default();
        let content: String = row
            .try_get::<Option<String>, _>("content")
            .ok()
            .flatten()
            .unwrap_or_default();
        match role.as_str() {
            "user" => out.push_str(&format!("## User\n\n{}\n\n", content)),
            "assistant" => out.push_str(&format!("## Assistant\n\n{}\n\n", content)),
            "tool" if include_tool_calls => {
                out.push_str(&format!("### Tool\n\n```\n{}\n```\n\n", content))
            }
            _ => {}
        }
    }
    Ok(out)
}
