use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct MessageSearchHit {
    pub message_id: String,
    pub conversation_id: String,
    pub conversation_title: String,
    pub conversation_mode: String,
    pub role: String,
    /// FTS5 snippet around the match, for the command palette row.
    pub excerpt: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Search every transcript message through the FTS5 index.
///
/// This is what makes the command palette a real search surface rather than a
/// title filter. Queries shorter than three characters (and queries with no
/// FTS5-safe run) return an empty list; the palette still filters titles
/// client-side for those.
#[tauri::command]
pub async fn search_messages(
    state: State<'_, AppState>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<MessageSearchHit>, String> {
    search_messages_with(state.storage.pool(), &query, limit).await
}

pub async fn search_messages_with(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    query: &str,
    limit: Option<u32>,
) -> Result<Vec<MessageSearchHit>, String> {
    let Some(match_query) = mixd_core::retrieval::fts_match_query(query) else {
        return Ok(Vec::new());
    };
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let rows: Vec<(String, String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT s.message_id, s.conversation_id, s.role, \
                snippet(message_search, 0, '', '', '…', 12), \
                m.created_at, COALESCE(json_extract(c.data, '$.title'), ''), \
                COALESCE(json_extract(c.data, '$.mode'), 'chat') \
         FROM message_search s \
         JOIN messages m ON m.id = s.message_id \
         JOIN conversations c ON c.id = s.conversation_id \
         WHERE message_search MATCH ? \
         ORDER BY bm25(message_search) LIMIT ?",
    )
    .bind(match_query)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(message_id, conversation_id, role, excerpt, created_at, title, mode)| {
                MessageSearchHit {
                    message_id,
                    conversation_id,
                    conversation_title: title,
                    conversation_mode: mode,
                    role,
                    excerpt,
                    created_at: parse_ts(&created_at),
                }
            },
        )
        .collect())
}

#[derive(Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub mode: String,
    pub model_id: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// The raw `updated_at` text used as the pagination cursor. Re-serialising
    /// the parsed timestamp could turn `…Z` into `…+00:00`, which compares
    /// differently in SQLite and would skip or repeat a row at the boundary.
    pub cursor: String,
    /// Latest message excerpt, shown in the thread sidebar.
    pub preview: Option<String>,
    pub message_count: i64,
    #[serde(rename = "project_id")]
    pub project_id: Option<String>,
    /// Stable workspace the conversation is bound to. Agent Runs, the browser
    /// profile and workspace-scoped permission rules all key on it, so it must
    /// not change between messages.
    pub workspace_id: Option<String>,
}

/// The shared `ConversationSummary` projection.
///
/// Every list below (paginated, by id, by project, title search, latest) must
/// return the same shape; duplicating this SQL is how `cursor`/preview drift
/// starts. `PREVIEW_CHARS` is interpolated because SQLite cannot bind a LIMIT
/// inside `substr`.
pub fn summary_select() -> String {
    format!(
        "SELECT c.id, json_extract(c.data, '$.title'), json_extract(c.data, '$.mode'), \
          json_extract(c.data, '$.modelId'), c.updated_at, \
          (SELECT substr(m.content, 1, {}) FROM messages m \
            WHERE m.conversation_id = c.id \
              AND m.role IN ('user', 'assistant') \
            ORDER BY m.rowid DESC LIMIT 1), \
          (SELECT count(*) FROM messages m WHERE m.conversation_id = c.id \
            AND m.role IN ('user', 'assistant')), \
          json_extract(c.data, '$.projectId'), \
          json_extract(c.data, '$.workspaceId') \
          FROM conversations c",
        limits::PREVIEW_CHARS
    )
}

pub fn summary_from_row(row: ConversationRow) -> ConversationSummary {
    let (id, title, mode, model_id, updated_at, preview, message_count, project_id, workspace_id) =
        row;
    let cursor = updated_at.clone();
    ConversationSummary {
        id,
        title,
        mode,
        model_id,
        updated_at: parse_ts(&updated_at),
        cursor,
        preview,
        message_count,
        project_id,
        workspace_id,
    }
}

/// One page of the thread sidebar plus whether another page exists.
#[derive(Serialize)]
pub struct ConversationPage {
    pub items: Vec<ConversationSummary>,
    pub has_more: bool,
}

/// Aggregate counts for the welcome screen.
///
/// The greeting used to derive these from `list_conversations`, which tied the
/// welcome card to the 200-row cap and to loading every summary row.
#[derive(Serialize)]
pub struct ConversationStats {
    pub count: i64,
    pub message_count: i64,
    pub last_activity_at: Option<String>,
}

/// A single thread, without materialising the sidebar page.
#[tauri::command]
pub async fn get_conversation(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<ConversationSummary>, String> {
    fetch_conversation_summary(state.storage.pool(), &id).await
}

/// Latest thread in any of `modes`, used to pick the initial surface thread.
#[tauri::command]
pub async fn latest_conversation(
    state: State<'_, AppState>,
    modes: Vec<String>,
) -> Result<Option<ConversationSummary>, String> {
    if modes.is_empty() {
        return Ok(None);
    }
    let placeholders = std::iter::repeat_n("?", modes.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "{} WHERE json_extract(c.data, '$.mode') IN ({placeholders}) \
         ORDER BY c.updated_at DESC, c.id DESC LIMIT 1",
        summary_select()
    );
    let mut query = sqlx::query_as::<_, ConversationRow>(&sql);
    for mode in &modes {
        query = query.bind(mode);
    }
    let row = query
        .fetch_optional(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(row.map(summary_from_row))
}

/// All threads linked to one project, newest first.
#[tauri::command]
pub async fn list_conversations_for_project(
    state: State<'_, AppState>,
    project_id: String,
    limit: Option<u32>,
) -> Result<ConversationPage, String> {
    let limit = limit.unwrap_or(500).clamp(1, 2_000);
    fetch_conversation_summaries_for_project(state.storage.pool(), &project_id, limit).await
}

/// Full-text title search for the command palette.
///
/// The palette used to filter `list_conversations` client-side, which silently
/// stopped at the 200-row cap. Searching in SQLite covers every thread without
/// shipping the whole history over IPC.
#[tauri::command]
pub async fn search_conversations(
    state: State<'_, AppState>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<ConversationSummary>, String> {
    search_conversation_summaries(state.storage.pool(), &query, limit).await
}

pub async fn search_conversation_summaries(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    query: &str,
    limit: Option<u32>,
) -> Result<Vec<ConversationSummary>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.unwrap_or(50).clamp(1, 100) as i64;
    let pattern = format!("%{}%", escape_like(query));
    let sql = format!(
        "{} WHERE json_extract(c.data, '$.title') LIKE ? ESCAPE '\\' \
         ORDER BY c.updated_at DESC, c.id DESC LIMIT ?",
        summary_select()
    );
    let rows: Vec<ConversationRow> = sqlx::query_as(&sql)
        .bind(pattern)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(rows.into_iter().map(summary_from_row).collect())
}

/// Total threads/messages and the most recent update, in one query.
#[tauri::command]
pub async fn conversation_stats(state: State<'_, AppState>) -> Result<ConversationStats, String> {
    fetch_conversation_stats(state.storage.pool()).await
}

pub async fn fetch_conversation_stats(
    pool: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<ConversationStats, String> {
    let row: (i64, i64, Option<String>) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM conversations), \
          (SELECT COUNT(*) FROM messages WHERE role IN ('user', 'assistant')), \
          (SELECT MAX(updated_at) FROM conversations)",
    )
    .fetch_one(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(ConversationStats {
        count: row.0,
        message_count: row.1,
        last_activity_at: row.2,
    })
}

pub fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Legacy, bounded summary list.
///
/// Kept for older clients; the sidebar uses [`list_conversations_page`], and
/// the project/approval/palette views use the dedicated lookup, search and
/// stats commands so none of them silently depends on this cap. It returns at
/// most [`limits::MAX_CONVERSATION_LIMIT`] rows.
#[tauri::command]
pub async fn list_conversations(
    state: State<'_, AppState>,
) -> Result<Vec<ConversationSummary>, String> {
    let pool = state.storage.pool();
    let page =
        fetch_conversation_summaries_page(pool, limits::MAX_CONVERSATION_LIMIT, None, None).await?;
    Ok(page.items)
}

/// Cursor page for the sidebar.
///
/// The cursor is `(updated_at, id)`: `updated_at` alone is not unique, and a
/// tie would either repeat or skip a thread when two conversations share a
/// timestamp. `id` is the stable tie-breaker.
#[tauri::command]
pub async fn list_conversations_page(
    state: State<'_, AppState>,
    limit: Option<u32>,
    before_updated_at: Option<String>,
    before_id: Option<String>,
) -> Result<ConversationPage, String> {
    let limit = limit
        .unwrap_or(limits::DEFAULT_CONVERSATION_LIMIT)
        .clamp(1, limits::MAX_CONVERSATION_LIMIT);
    fetch_conversation_summaries_page(
        state.storage.pool(),
        limit,
        before_updated_at.as_deref(),
        before_id.as_deref(),
    )
    .await
}

/// Query helper kept separate from the command so integration tests can call
/// it against a temporary `Storage` without a Tauri `State` wrapper.
pub async fn fetch_conversation_summaries(
    pool: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Vec<ConversationSummary>, String> {
    Ok(
        fetch_conversation_summaries_page(pool, u32::MAX, None, None)
            .await?
            .items,
    )
}

pub async fn fetch_conversation_summaries_page(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    limit: u32,
    before_updated_at: Option<&str>,
    before_id: Option<&str>,
) -> Result<ConversationPage, String> {
    // Fetch one extra row to answer `has_more` without a second COUNT query.
    let fetch = limit.saturating_add(1).min(i64::MAX as u32) as i64;
    let sql = format!(
        "{} WHERE ?1 IS NULL \
             OR c.updated_at < ?1 \
             OR (c.updated_at = ?1 AND c.id < ?2) \
          ORDER BY c.updated_at DESC, c.id DESC \
          LIMIT ?3",
        summary_select()
    );
    let rows: Vec<ConversationRow> = sqlx::query_as(&sql)
        .bind(before_updated_at)
        .bind(before_id.unwrap_or(""))
        .bind(fetch)
        .fetch_all(pool)
        .await
        .map_err(crate::error::cmd_err)?;

    let has_more = rows.len() > limit as usize;
    let rows = if has_more {
        rows.into_iter().take(limit as usize).collect()
    } else {
        rows
    };
    Ok(ConversationPage {
        items: rows.into_iter().map(summary_from_row).collect(),
        has_more,
    })
}

pub async fn fetch_conversation_summary(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    id: &str,
) -> Result<Option<ConversationSummary>, String> {
    let sql = format!("{} WHERE c.id = ?", summary_select());
    let row: Option<ConversationRow> = sqlx::query_as(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(row.map(summary_from_row))
}

pub async fn fetch_conversation_summaries_for_project(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    project_id: &str,
    limit: u32,
) -> Result<ConversationPage, String> {
    let fetch = limit.saturating_add(1).min(i64::MAX as u32) as i64;
    let sql = format!(
        "{} WHERE json_extract(c.data, '$.projectId') = ? \
          ORDER BY c.updated_at DESC, c.id DESC LIMIT ?",
        summary_select()
    );
    let rows: Vec<ConversationRow> = sqlx::query_as(&sql)
        .bind(project_id)
        .bind(fetch)
        .fetch_all(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    let has_more = rows.len() > limit as usize;
    let rows = if has_more {
        rows.into_iter().take(limit as usize).collect()
    } else {
        rows
    };
    Ok(ConversationPage {
        items: rows.into_iter().map(summary_from_row).collect(),
        has_more,
    })
}

pub type ConversationRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    Option<String>,
    Option<String>,
);

pub type ConversationDispatchRow = (String, String, String, String, String, Option<String>);

pub fn parse_ts(value: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| {
            // A corrupt timestamp must not masquerade as "just now": `Utc::now`
            // sorted the broken row above every healthy thread and hid the
            // corruption. Epoch sorts it last, where it is visible but harmless.
            tracing::warn!(timestamp = %value, "unparseable timestamp, falling back to epoch");
            chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap_or_else(Utc::now)
        })
}
