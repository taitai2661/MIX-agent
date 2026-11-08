use super::*;

#[derive(Deserialize, Clone, Serialize)]
pub struct AttachmentInput {
    pub kind: String,
    pub mime_type: String,
    pub data_base64: String,
    pub filename: Option<String>,
}

#[derive(Deserialize)]
pub struct SendMessageInput {
    pub conversation_id: String,
    pub content: String,
    pub model_id: Option<String>,
    pub mode: Option<String>,
    pub workspace_id: Option<String>,
    pub attachments: Vec<AttachmentInput>,
    pub reasoning: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Docker-edition "temporary mode": no Memory retrieval, and the turn is
    /// purged once the Run ends.
    #[serde(default)]
    pub temporary_mode: bool,
    /// Only meaningful with `temporary_mode`: temporary runs expose no tools
    /// unless this is set, and even then Memory / Skill tools stay out.
    #[serde(default)]
    pub allow_tools: bool,
}

#[derive(Serialize)]
pub struct SendMessageOutput {
    pub run_id: String,
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, AppState>,
    input: SendMessageInput,
) -> Result<SendMessageOutput, String> {
    let conversation_id = input.conversation_id.clone();
    let output = dispatch_run(&state, input).await?;
    // Index the new user turn immediately; the Run's terminal event will
    // refresh the assistant preview/title when it finishes.
    state
        .spotlight
        .schedule(SpotlightKind::Conversation, &conversation_id)
        .await;
    Ok(output)
}

#[derive(Deserialize)]
pub struct EditMessageInput {
    pub conversation_id: String,
    pub message_id: String,
    pub content: String,
    pub model_id: Option<String>,
    pub mode: Option<String>,
    pub reasoning: Option<String>,
}

/// Edit a user turn and regenerate from it.
///
/// This is a "branch from here": the edited message and every message after it
/// are removed, together with the Runs their transcripts belonged to, and the
/// new text is dispatched as a fresh turn. Later turns are discarded rather
/// than kept because they answered the old wording — keeping them would leave
/// the conversation claiming the model replied to something the user never
/// sent.
#[tauri::command]
pub async fn edit_message(
    state: State<'_, AppState>,
    input: EditMessageInput,
) -> Result<SendMessageOutput, String> {
    let content = input.content.trim().to_string();
    if content.is_empty() {
        return Err("メッセージを入力してください".to_string());
    }
    let pool = state.storage.pool();

    // Locate the target by both id and conversation so a stray id from another
    // thread cannot be used to rewrite history. `created_at` + `rowid` is the
    // same ordering the transcript uses, so "onward" means what the UI shows.
    let target: Option<(String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT json_extract(data, '$.role'), created_at, rowid, \
                json_extract(data, '$.attachments') \
         FROM messages WHERE id = ? AND conversation_id = ?",
    )
    .bind(&input.message_id)
    .bind(&input.conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    let Some((role, created_at, rowid, attachments_raw)) = target else {
        return Err("メッセージが見つかりません".to_string());
    };
    if role != "user" {
        return Err("ユーザーのメッセージのみ編集できます".to_string());
    }

    // Attachments are not editable in the transcript yet, but dropping them
    // because the text changed would silently turn an image question into a
    // text question, so the originals are carried into the replacement turn.
    // A stored attachment that no longer parses must not be dropped silently
    // either: refuse the edit rather than send the text without its image.
    let attachments: Vec<AttachmentInput> = match attachments_raw.as_deref() {
        None => Vec::new(),
        Some(raw) => serde_json::from_str(raw).map_err(|err| {
            tracing::warn!(
                %err,
                message_id = %input.message_id,
                "could not parse the edited message's attachments"
            );
            "元のメッセージの添付を読み込めませんでした".to_string()
        })?,
    };

    // The in-flight Run guard, the truncation and the replacement Run all live
    // in one transaction inside `dispatch`, so a Run cannot start against a
    // deleted tail and a failed dispatch cannot leave the transcript empty.
    let conversation_id = input.conversation_id.clone();
    let output = dispatch(
        &state,
        SendMessageInput {
            conversation_id: input.conversation_id,
            content,
            model_id: input.model_id,
            mode: input.mode,
            workspace_id: None,
            attachments,
            reasoning: input.reasoning,
            temperature: None,
            max_tokens: None,
            temporary_mode: false,
            allow_tools: false,
        },
        Some(TruncateTail { created_at, rowid }),
    )
    .await?;
    state
        .spotlight
        .schedule(SpotlightKind::Conversation, &conversation_id)
        .await;
    Ok(output)
}

/// Delete every row that hangs off a Run, in foreign-key dependency order.
///
/// `PRAGMA foreign_keys` is ON for every sqlx 0.8 connection (see
/// `storage/tests/fk.rs`), and the schema declares `approvals.tool_call_id →
/// tool_calls`, `tool_calls.run_id → runs` and friends. Deleting `runs` (or
/// `tool_calls` first) therefore fails with `FOREIGN KEY constraint failed`
/// for any run that ever made a tool call.
///
/// `scheduled_runs.run_id` and `memory_action_events.run_id` are nullable and
/// carry history of their own, so they are unlinked rather than deleted;
/// `memory_processing_jobs.run_id` is `NOT NULL` and has nothing to say once
/// the run is gone, so it goes.
///
/// Takes a connection so the caller can compose this inside their own
/// transaction.
pub async fn delete_run_cascade(
    conn: &mut sqlx::SqliteConnection,
    run_id: &str,
) -> Result<(), String> {
    // The rollback ref is anchored in the workspace repository, so it must be
    // dropped while the `runs` row still says which repository that is.
    crate::workspace_baseline::forget_ref(conn, run_id).await;
    // Leaf-first: approvals reference tool_calls, both reference runs.
    for (table, column) in [
        ("approvals", "run_id"),
        ("tool_calls", "run_id"),
        ("run_events", "run_id"),
        ("run_checkpoints", "run_id"),
        ("memory_processing_jobs", "run_id"),
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE {column} = ?"))
            .bind(run_id)
            .execute(&mut *conn)
            .await
            .map_err(crate::error::cmd_err)?;
    }
    for (table, column) in [
        ("scheduled_runs", "run_id"),
        ("memory_action_events", "run_id"),
    ] {
        sqlx::query(&format!(
            "UPDATE {table} SET {column} = NULL WHERE {column} = ?"
        ))
        .bind(run_id)
        .execute(&mut *conn)
        .await
        .map_err(crate::error::cmd_err)?;
    }
    sqlx::query("DELETE FROM runs WHERE id = ?")
        .bind(run_id)
        .execute(&mut *conn)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(())
}

/// Erase every trace of a temporary Run: its transcript, the attachments and
/// images embedded in those message rows, its tool calls, events, checkpoints
/// and finally the Run row itself.
///
/// When `delete_conversation` is set the conversation was created for this one
/// turn (it had no earlier messages), so it is removed as well. Otherwise the
/// surrounding thread is kept and only this turn disappears.
///
/// Messages are matched on the `runId` the Engine stamps into every persisted
/// row, so a concurrent or later turn in the same conversation is untouched.
pub async fn purge_temporary_run(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    conversation_id: &str,
    run_id: &str,
    delete_conversation: bool,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(crate::error::cmd_err)?;

    delete_run_cascade(&mut tx, run_id).await?;

    if delete_conversation {
        // The whole thread goes away: remove every message (including
        // activity rows and legacy rows with no `runId`) and their feedback
        // first, otherwise the conversation delete hits a FOREIGN KEY failure
        // or leaves orphans behind.
        sqlx::query(
            "DELETE FROM message_feedback WHERE message_id IN \
             (SELECT id FROM messages WHERE conversation_id = ?)",
        )
        .bind(conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;

        sqlx::query("DELETE FROM messages WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&mut *tx)
            .await
            .map_err(crate::error::cmd_err)?;

        sqlx::query("DELETE FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .execute(&mut *tx)
            .await
            .map_err(crate::error::cmd_err)?;

        tx.commit().await.map_err(crate::error::cmd_err)?;
        return Ok(());
    }

    // `message_feedback.message_id` → `messages.id`, so feedback goes first.
    sqlx::query(
        "DELETE FROM message_feedback WHERE message_id IN \
         (SELECT id FROM messages WHERE conversation_id = ? AND json_extract(data, '$.runId') = ?)",
    )
    .bind(conversation_id)
    .bind(run_id)
    .execute(&mut *tx)
    .await
    .map_err(crate::error::cmd_err)?;

    sqlx::query(
        "DELETE FROM messages WHERE conversation_id = ? AND json_extract(data, '$.runId') = ?",
    )
    .bind(conversation_id)
    .bind(run_id)
    .execute(&mut *tx)
    .await
    .map_err(crate::error::cmd_err)?;

    tx.commit().await.map_err(crate::error::cmd_err)?;
    Ok(())
}

/// Delete the target message and everything chronologically after it, plus the
/// Runs whose transcripts those messages made up.
///
/// Runs are keyed to the conversation, not the message, so their rows would
/// otherwise survive with no transcript and keep showing up in the activity
/// panel with nothing behind them. `message_feedback`, run events, checkpoints,
/// tool calls and approvals reference those rows and are cleared with them by
/// [`delete_run_cascade`] — they must be, because the foreign keys *are*
/// enforced.
///
/// `pub` so integration tests can exercise the branch-and-regenerate
/// truncation without standing up the full `AppState`.
///
/// Takes the caller's connection so the deletion runs inside a transaction the
/// caller controls: a failure part-way through leaves the transcript untouched
/// rather than half-deleted.
pub async fn truncate_messages_from(
    conn: &mut sqlx::SqliteConnection,
    conversation_id: &str,
    created_at: &str,
    rowid: i64,
) -> Result<(), String> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT json_extract(data, '$.runId') AS run_id FROM messages \
         WHERE conversation_id = ? \
         AND (created_at > ? OR (created_at = ? AND rowid >= ?))",
    )
    .bind(conversation_id)
    .bind(created_at)
    .bind(created_at)
    .bind(rowid)
    .fetch_all(&mut *conn)
    .await
    .map_err(crate::error::cmd_err)?;
    let run_ids: Vec<String> = rows
        .iter()
        .filter_map(|row| row.try_get::<String, _>("run_id").ok())
        .collect();
    // `message_feedback.message_id` references `messages(id)`: it has to go
    // before the messages it is attached to, or the delete is rejected.
    sqlx::query(
        "DELETE FROM message_feedback WHERE message_id IN \
         (SELECT id FROM messages WHERE conversation_id = ? \
          AND (created_at > ? OR (created_at = ? AND rowid >= ?)))",
    )
    .bind(conversation_id)
    .bind(created_at)
    .bind(created_at)
    .bind(rowid)
    .execute(&mut *conn)
    .await
    .map_err(crate::error::cmd_err)?;
    for run_id in &run_ids {
        delete_run_cascade(conn, run_id).await?;
    }
    sqlx::query(
        "DELETE FROM messages WHERE conversation_id = ? \
         AND (created_at > ? OR (created_at = ? AND rowid >= ?))",
    )
    .bind(conversation_id)
    .bind(created_at)
    .bind(created_at)
    .bind(rowid)
    .execute(&mut *conn)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(())
}
