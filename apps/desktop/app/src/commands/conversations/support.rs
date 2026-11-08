use super::*;

pub fn attachment_token_estimate(attachment: &AttachmentInput) -> u64 {
    if attachment.mime_type.starts_with("image/") || attachment.kind == "image" {
        1_000
    } else if attachment.mime_type == "application/pdf" {
        2_000
    } else {
        (attachment.data_base64.len() as u64 / 4).min(100_000)
    }
}

/// Start a Run for a user turn.
///
/// Takes `&AppState` rather than `State` so the scheduler can start background
pub fn validate_send_input(input: &SendMessageInput) -> Result<(), String> {
    if input.conversation_id.trim().is_empty() {
        return Err("conversation_id must not be empty".to_string());
    }
    if Uuid::parse_str(&input.conversation_id).is_err() {
        return Err("invalid conversation_id".to_string());
    }
    let trimmed = input.content.trim();
    if trimmed.is_empty() {
        return Err("メッセージを入力してください".to_string());
    }
    if input.content.chars().count() > limits::MAX_CONTENT_CHARS {
        return Err(format!(
            "message too long (max {} chars)",
            limits::MAX_CONTENT_CHARS
        ));
    }
    if let Some(model) = &input.model_id {
        if model.trim().is_empty() {
            return Err("model_id must not be empty".to_string());
        }
        if model.chars().count() > 256 {
            return Err("model_id too long".to_string());
        }
    }
    if let Some(mode) = &input.mode {
        if mode.trim().is_empty() {
            return Err("mode must not be empty".to_string());
        }
        mode.parse::<mixd_core::Mode>()
            .map_err(|_| "invalid mode".to_string())?;
    }
    if let Some(workspace) = &input.workspace_id {
        if Uuid::parse_str(workspace).is_err() {
            return Err("invalid workspace_id".to_string());
        }
    }
    if let Some(reasoning) = &input.reasoning {
        match reasoning.as_str() {
            "low" | "medium" | "high" | "xhigh" => {}
            _ => return Err("invalid reasoning level".to_string()),
        }
    }
    if let Some(temp) = input.temperature {
        if !temp.is_finite() || !(0.0..=2.0).contains(&temp) {
            return Err("temperature must be a finite number between 0 and 2".to_string());
        }
    }
    if let Some(max) = input.max_tokens {
        if max == 0 || max > 128_000 {
            return Err("max_tokens must be between 1 and 128000".to_string());
        }
    }
    if input.attachments.len() > limits::MAX_ATTACHMENTS {
        return Err(format!(
            "too many attachments (max {})",
            limits::MAX_ATTACHMENTS
        ));
    }
    let mut total: usize = 0;
    for attachment in &input.attachments {
        if attachment.kind.trim().is_empty() || attachment.mime_type.trim().is_empty() {
            return Err("attachment kind and mime_type must not be empty".to_string());
        }
        if attachment.mime_type.chars().count() > 128 {
            return Err("attachment mime_type too long".to_string());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&attachment.data_base64)
            .map_err(|_| "attachment data_base64 is not valid base64".to_string())?;
        if bytes.len() > limits::MAX_ATTACHMENT_BYTES {
            return Err(format!(
                "attachment too large (max {} bytes)",
                limits::MAX_ATTACHMENT_BYTES
            ));
        }
        total += bytes.len();
        if total > limits::MAX_ATTACHMENTS_TOTAL_BYTES {
            return Err(format!(
                "attachments too large in total (max {} bytes)",
                limits::MAX_ATTACHMENTS_TOTAL_BYTES
            ));
        }
    }
    Ok(())
}

/// Persist a failed Run and emit its terminal events.
///
/// Shared by the `Err` and `JoinError` arms of the run-task join: the two used
/// to spell the same `UPDATE runs SET status = 'failed' …` plus `error` +
pub async fn record_run_failed(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    engine: &std::sync::Arc<mixd_core::runs::engine::RunEngine>,
    run_id: RunId,
    message: &str,
) {
    // Persist the reason into `data` as well as the status column:
    // `run_view_from_row` reads `$.error`, and without it the UI
    // (and any later history read) could not explain the failure.
    mixd_storage::log_write(
        sqlx::query(
            "UPDATE runs SET status = 'failed', \
              data = json_set(data, '$.status', 'failed', '$.error', ?, \
                               '$.updatedAt', ?) \
              WHERE id = ?",
        )
        .bind(message)
        .bind(Utc::now().to_rfc3339())
        .bind(run_id.to_string())
        .execute(pool)
        .await,
        "record failed run status",
    );
    // The engine bailed before emitting anything, so the UI has no
    // terminal event to key `isTerminal()` on and the spinner would
    // run until the window was reloaded.
    engine
        .emit(run_id, "error", serde_json::json!({ "error": message }))
        .await;
    engine
        .emit(
            run_id,
            "status",
            serde_json::json!({ "status": "failed", "error": message }),
        )
        .await;
}

pub async fn resolve_provider_id(state: &AppState, model_id: &str) -> Result<String, String> {
    // Model ids are stored as `<provider_id>:<model>` when auto-discovered, and
    // bare model names otherwise. The shared resolver owns both the prefix
    // lookup and the lazy re-hydration of an adapter the registry is missing;
    // see `crate::resolve_provider_for_model`.
    crate::resolve_provider_for_model(
        &state.engine.providers,
        state.storage.as_ref(),
        state.secrets.as_ref(),
        model_id,
    )
    .await
    .map_err(crate::error::cmd_err)
}

pub async fn load_history<'e, E>(
    executor: E,
    conversation_id: &str,
) -> Result<Vec<HistoryMessage>, String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    use sqlx::Row;
    // `toolCalls` has to come back. It used to be selected but hard-coded to an
    // empty vector, so a reloaded conversation handed the model a `tool` message
    // whose originating assistant turn carried no `tool_calls`. OpenAI answers
    // 400 for that ("messages with role 'tool' must be a response to a preceding
    // message with 'tool_calls'"), so any conversation that had used a tool was
    // permanently broken from the second message onwards.
    // The *newest* 80, then reversed. Keeping the oldest 80 meant that past
    // that point every later turn was built from the head of the conversation
    // and the model lost all recent context. `rowid` breaks ties so messages
    // written in the same millisecond keep their order.
    let rows = sqlx::query(
        "SELECT data FROM messages WHERE conversation_id = ? \
         ORDER BY created_at DESC, rowid DESC LIMIT ?",
    )
    .bind(conversation_id)
    .bind(limits::HISTORY_LIMIT)
    .fetch_all(executor)
    .await
    .map_err(crate::error::cmd_err)?;
    let rows: Vec<_> = rows.into_iter().rev().collect();
    let fetched = rows.len();
    // Dropped rows are counted and warned, not silently skipped: a corrupt
    // message row used to vanish from history with no trace, and the next
    // turn was built as if it had never existed.
    let mut skipped_activity = 0u32;
    let history: Vec<HistoryMessage> = rows
        .into_iter()
        .filter_map(|row| {
            let raw: String = row.try_get("data").ok()?;
            let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
            if value.get("kind").and_then(|v| v.as_str()) == Some("activity") {
                skipped_activity += 1;
                return None;
            }
            let message = HistoryMessage {
                role: value.get("role")?.as_str()?.to_string(),
                content: value
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                tool_call_id: value
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                name: value
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                tool_calls: value
                    .get("toolCalls")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default(),
                is_error: value
                    .get("isError")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                native: value
                    .get("native")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default(),
            };
            Some(message)
        })
        .collect();
    // `filter_map` above cannot distinguish "activity row" from "corrupt";
    // recount corrupt rows as fetched-minus-kept-minus-activity.
    let dropped = fetched.saturating_sub(history.len() + skipped_activity as usize) as u32;
    if dropped > 0 {
        tracing::warn!(
            conversation_id,
            dropped,
            "load_history skipped corrupt message rows"
        );
    }
    Ok(history)
}

pub async fn build_tool_specs(state: &AppState, mode: mixd_core::Mode) -> Arc<Vec<ToolSpec>> {
    // Docker parity: chat runs carry tools too (read/search/file/browser/MCP),
    // minus delegation (`agent`) and long-work planning (`todo`, `user_plan`).
    // Thinking additionally keeps the checklist; agent keeps everything.
    // Shared by the run's turns rather than rebuilt per turn.
    // Typed as `Mode` (not `&str`): the parse boundary in `dispatch` rejects
    // unknown modes with a warning instead of silently degrading the tool set.
    let specs = state.engine.tool_specs_for_mode(mode).await;
    Arc::new(specs)
}
