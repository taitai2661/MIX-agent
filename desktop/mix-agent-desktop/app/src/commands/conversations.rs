use std::sync::Arc;

use crate::AppState;
use chrono::Utc;
use mixd_core::runs::engine::{HistoryMessage, RunRequest};
use mixd_core::runs::state::RunId;
use mixd_providers::{ReasoningControl, ToolSpec};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub mode: String,
    pub model_id: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
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

#[tauri::command]
pub async fn list_conversations(
    state: State<'_, AppState>,
) -> Result<Vec<ConversationSummary>, String> {
    let pool = state.storage.pool();
    let rows = fetch_conversation_summaries(pool).await?;
    Ok(rows)
}

/// Query helper kept separate from the command so integration tests can call
/// it against a temporary `Storage` without a Tauri `State` wrapper.
pub async fn fetch_conversation_summaries(
    pool: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Vec<ConversationSummary>, String> {
    let rows: Vec<ConversationRow> = sqlx::query_as(
        "SELECT c.id, json_extract(c.data, '$.title'), json_extract(c.data, '$.mode'), \
          json_extract(c.data, '$.modelId'), json_extract(c.data, '$.updatedAt'), \
          (SELECT substr(json_extract(m.data, '$.content'), 1, 140) FROM messages m \
            WHERE m.conversation_id = c.id ORDER BY m.created_at DESC LIMIT 1), \
          (SELECT count(*) FROM messages m WHERE m.conversation_id = c.id), \
          json_extract(c.data, '$.projectId'), \
          json_extract(c.data, '$.workspaceId') \
          FROM conversations c ORDER BY json_extract(c.data, '$.updatedAt') DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, title, mode, model_id, updated_at, preview, message_count, project_id, workspace_id)| {
                ConversationSummary {
                    id,
                    title,
                    mode,
                    model_id,
                    updated_at: parse_ts(&updated_at),
                    preview,
                    message_count,
                    project_id,
                    workspace_id,
                }
            },
        )
        .collect())
}

type ConversationRow = (
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

type ConversationDispatchRow = (String, String, String, String, String, Option<String>);

fn parse_ts(value: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[derive(Deserialize)]
pub struct ConversationCreate {
    pub mode: String,
    pub model_id: String,
    pub title: Option<String>,
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}

async fn project_exists(pool: &sqlx::SqlitePool, id: &str) -> Result<bool, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT id FROM projects WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(row.is_some())
}

#[tauri::command]
pub async fn create_conversation(
    state: State<'_, AppState>,
    input: ConversationCreate,
) -> Result<ConversationSummary, String> {
    let pool = state.storage.pool();
    let project_id = input.project_id.filter(|p| !p.trim().is_empty());
    if let Some(pid) = &project_id {
        if !project_exists(pool, pid).await? {
            return Err("プロジェクトが見つかりません".to_string());
        }
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let title = input.title.unwrap_or_else(|| "新しい会話".to_string());
    // The conversation owns its workspace from creation. The agent's browser
    // profile and workspace-scoped permission rules key on it, so it must not
    // be regenerated per message.
    let workspace_id = input
        .workspace_id
        .clone()
        .filter(|w| Uuid::parse_str(w).is_ok())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let data = serde_json::json!({
        "title": title,
        "mode": input.mode,
        "modelId": input.model_id,
        "workspaceId": workspace_id.clone(),
        "projectId": project_id,
        "updatedAt": now.to_rfc3339(),
        "state": "active",
    });
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(ConversationSummary {
        id,
        title,
        mode: input.mode,
        model_id: input.model_id,
        updated_at: now,
        preview: None,
        message_count: 0,
        project_id,
        workspace_id: Some(workspace_id),
    })
}

/// Attach / detach a conversation to a project after creation.
#[tauri::command]
pub async fn set_conversation_project(
    state: State<'_, AppState>,
    conversation_id: String,
    project_id: Option<String>,
) -> Result<(), String> {
    let pool = state.storage.pool();
    let project_id = project_id.filter(|p| !p.trim().is_empty());
    if let Some(pid) = &project_id {
        if !project_exists(pool, pid).await? {
            return Err("プロジェクトが見つかりません".to_string());
        }
    }
    sqlx::query("UPDATE conversations SET data = json_set(data, '$.projectId', ?) WHERE id = ?")
        .bind(project_id)
        .bind(&conversation_id)
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(())
}

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
    dispatch_run(&state, input).await
}

/// Start a Run for a user turn.
///
/// Takes `&AppState` rather than `State` so the scheduler can start background
/// Runs without going through a Tauri command.
pub async fn dispatch_run(
    state: &AppState,
    input: SendMessageInput,
) -> Result<SendMessageOutput, String> {
    let pool = state.storage.pool();
    let row: Option<ConversationDispatchRow> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.mode'), json_extract(data, '$.modelId'), \
          json_extract(data, '$.title'), json_extract(data, '$.updatedAt'), \
          json_extract(data, '$.workspaceId') \
          FROM conversations WHERE id = ?",
    )
    .bind(&input.conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    let (_, stored_mode, stored_model, _, _, stored_workspace) =
        row.ok_or_else(|| "conversation not found".to_string())?;

    // A mode passed by the caller (the composer's mode selector) overrides the
    // mode stored at conversation creation; without this the selector was
    // silently ignored for existing conversations.
    let mode_str = input
        .mode
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| stored_mode.clone());
    let mode = mixd_core::Mode::parse(&mode_str).ok_or("invalid mode")?;
    if mode_str != stored_mode {
        let _ =
            sqlx::query("UPDATE conversations SET data = json_set(data, '$.mode', ?) WHERE id = ?")
                .bind(&mode_str)
                .bind(&input.conversation_id)
                .execute(pool)
                .await;
    }
    let model_id = input.model_id.clone().unwrap_or(stored_model);
    let provider_id = resolve_provider_id(state, &model_id).await?;
    // Prefer an explicitly requested workspace, then the one the conversation
    // already owns, and only mint a fresh id for a conversation created before
    // workspaces were persisted. Regenerating it on every message (the old
    // behaviour) silently gave the agent a new browser profile, terminal
    // session and permission scope each turn.
    let stored_workspace = stored_workspace.and_then(|w| Uuid::parse_str(&w).ok());
    let requested_workspace = input
        .workspace_id
        .clone()
        .and_then(|w| Uuid::parse_str(&w).ok());
    let workspace_id = requested_workspace
        .or(stored_workspace)
        .unwrap_or_else(Uuid::new_v4);
    if stored_workspace != Some(workspace_id) {
        // Persist it back so the browser pane and later Runs resolve the same
        // profile.
        let _ = sqlx::query(
            "UPDATE conversations SET data = json_set(data, '$.workspaceId', ?) WHERE id = ?",
        )
        .bind(workspace_id.to_string())
        .bind(&input.conversation_id)
        .execute(pool)
        .await;
    }

    let run_id = RunId::new_v4();
    let now = Utc::now();
    let budget = serde_json::json!({
        "stepLimit": mode.default_step_limit(),
        "toolCallLimit": mode.default_tool_call_limit(),
        "wallClockMinutes": mode.default_wall_clock_minutes(),
        "grantsUsed": 0,
        "maxGrants": 2,
    });
    let request_key = format!("conv:{}:{}", input.conversation_id, now.timestamp_millis());
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?, ?, ?, ?, 'queued', ?)",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(
        serde_json::json!({
            "mode": mode.as_str(),
            "modelId": model_id,
            "workspaceId": workspace_id.to_string(),
            "status": "queued",
            "stepCount": 0,
            "toolCallCount": 0,
            "startedAt": now.to_rfc3339(),
            "updatedAt": now.to_rfc3339(),
            "summary": "",
            "budget": budget,
        })
        .to_string(),
    )
    .bind(&input.conversation_id)
    .bind(&request_key)
    .execute(pool)
    .await
    .map_err(crate::error::cmd_err)?;

    // Before the user turn is inserted. `load_history` selects every message,
    // so loading it afterwards included the row just written and the engine
    // appended the same text again — every turn was sent to the model twice.
    let history: Vec<HistoryMessage> = load_history(pool, &input.conversation_id).await?;

    // Persist the user turn.
    let user_data = serde_json::json!({
        "role": "user",
        "content": input.content,
        "modelId": model_id,
        "runId": run_id.to_string(),
        "toolCalls": [],
        "attachments": input.attachments,
        "kind": "user",
    });
    sqlx::query("INSERT INTO messages (id, owner_id, data, conversation_id) VALUES (?, ?, ?, ?)")
        .bind(Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(user_data.to_string())
        .bind(&input.conversation_id)
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;

    // Touch the conversation so it sorts to the top.
    mixd_storage::log_write(
        sqlx::query(
            "UPDATE conversations SET data = json_set(data, '$.updatedAt', ?) WHERE id = ?",
        )
        .bind(now.to_rfc3339())
        .bind(&input.conversation_id)
        .execute(pool)
        .await,
        "touch conversation",
    );

    // Project context: instructions / sources / AGENTS.md → system prompt,
    // and the project folder → tool workspace root.
    let project = super::projects::load_project_runtime(pool, &input.conversation_id).await?;

    let tools = build_tool_specs(state, mode.as_str()).await;
    let reasoning = match input.reasoning.as_deref() {
        Some("low") => Some(ReasoningControl::Low),
        Some("medium") => Some(ReasoningControl::Medium),
        Some("high") => Some(ReasoningControl::High),
        Some("xhigh") => Some(ReasoningControl::Xhigh),
        _ => None,
    };

    let request = RunRequest {
        run_id,
        conversation_id: Uuid::parse_str(&input.conversation_id).map_err(crate::error::cmd_err)?,
        workspace_id,
        root_path: project.root_path,
        additional_paths: project.additional_paths,
        mode,
        model_id: model_id.clone(),
        provider_id,
        system: project.system,
        temperature: input.temperature,
        reasoning,
        max_tokens: input.max_tokens,
        tools,
        history,
        user_message: input.content,
        resume_from: None,
    };

    let engine = state.engine.clone();
    let pool = pool.clone();
    let conversation_id = input.conversation_id.clone();
    tokio::spawn(async move {
        let result = engine.execute(request).await;
        match result {
            Ok(outcome) => {
                mixd_storage::log_write(
                    sqlx::query("UPDATE runs SET status = ? WHERE id = ?")
                        .bind(outcome.status.as_str())
                        .bind(run_id.to_string())
                        .execute(&pool)
                        .await,
                    "record final run status",
                );
                // The assistant turn is already in `messages`: the engine writes
                // the whole transcript, including tool-call turns and results,
                // because it is the only place that knows the correct
                // interleaving. Writing the final text again here would duplicate
                // it and drop the tool history that a later turn needs.
                let _ = (conversation_id, model_id, outcome.reasoning);
            }
            Err(err) => {
                tracing::error!(?err, %run_id, "run failed");
                let message = err.to_string();
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
                    .bind(&message)
                    .bind(Utc::now().to_rfc3339())
                    .bind(run_id.to_string())
                    .execute(&pool)
                    .await,
                    "record failed run status",
                );
                // The engine bailed before emitting anything, so the UI has no
                // terminal event to key `isTerminal()` on and the spinner would
                // run until the window was reloaded.
                engine
                    .emit(run_id, "error", serde_json::json!({ "error": message.clone() }))
                    .await;
                engine
                    .emit(
                        run_id,
                        "status",
                        serde_json::json!({ "status": "failed", "error": message }),
                    )
                    .await;
            }
        }
    });

    Ok(SendMessageOutput {
        run_id: run_id.to_string(),
    })
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
    let attachments: Vec<AttachmentInput> = match attachments_raw.as_deref() {
        None => Vec::new(),
        Some(raw) => match serde_json::from_str(raw) {
            Ok(parsed) => parsed,
            Err(err) => {
                // A stored attachment that no longer parses would otherwise be
                // dropped without a trace; log it so the loss is diagnosable.
                tracing::warn!(
                    %err,
                    message_id = %input.message_id,
                    "could not parse the edited message's attachments; carrying none"
                );
                Vec::new()
            }
        },
    };

    // The engine appends assistant turns to a transcript this command is about
    // to rewrite, so an in-flight Run must finish (or be cancelled) first. The
    // check and the truncation share one transaction so a Run cannot start in
    // the window between them and have its transcript deleted under it.
    let mut tx = pool.begin().await.map_err(crate::error::cmd_err)?;
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs WHERE conversation_id = ? AND status IN \
         ('queued','running','waiting_approval','budget_extension_pending')",
    )
    .bind(&input.conversation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(crate::error::cmd_err)?;
    if active > 0 {
        return Err("実行中はメッセージを編集できません".to_string());
    }

    truncate_messages_from(&mut tx, &input.conversation_id, &created_at, rowid).await?;
    tx.commit().await.map_err(crate::error::cmd_err)?;

    dispatch_run(
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
        },
    )
    .await
}

/// Delete the target message and everything chronologically after it, plus the
/// Runs whose transcripts those messages made up.
///
/// Runs are keyed to the conversation, not the message, so their rows would
/// otherwise survive with no transcript and keep showing up in the activity
/// panel with nothing behind them. `run_events`, checkpoints, tool calls and
/// approvals reference a Run and are cleared with it (SQLite does not enforce
/// those foreign keys here).
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
    for row in &rows {
        let run_id: Option<String> = row.try_get("run_id").ok().flatten();
        let Some(run_id) = run_id else { continue };
        for (table, column) in [
            ("run_events", "run_id"),
            ("run_checkpoints", "run_id"),
            ("tool_calls", "run_id"),
            ("approvals", "run_id"),
            ("runs", "id"),
        ] {
            sqlx::query(&format!("DELETE FROM {table} WHERE {column} = ?"))
                .bind(&run_id)
                .execute(&mut *conn)
                .await
                .map_err(crate::error::cmd_err)?;
        }
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

async fn resolve_provider_id(state: &AppState, model_id: &str) -> Result<String, String> {
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
    .map_err(|err| err.to_string())
}

async fn load_history(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    conversation_id: &str,
) -> Result<Vec<HistoryMessage>, String> {
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
         ORDER BY created_at DESC, rowid DESC LIMIT 80",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    let rows: Vec<_> = rows.into_iter().rev().collect();
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let raw: String = row.try_get("data").ok()?;
            let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
            if value.get("kind").and_then(|v| v.as_str()) == Some("activity") {
                return None;
            }
            Some(HistoryMessage {
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
            })
        })
        .collect())
}

async fn build_tool_specs(state: &AppState, mode: &str) -> Arc<Vec<ToolSpec>> {
    // Chat mode answers directly; Thinking and Agent may use Tools.
    if mode == "chat" {
        return Arc::new(Vec::new());
    }
    // Shared by the run's turns rather than rebuilt per turn.
    Arc::new(state.engine.tool_specs().await)
}

#[tauri::command]
pub async fn delete_conversation(state: State<'_, AppState>, id: String) -> Result<(), String> {
    // `messages`, `runs` and `run_events` reference the conversation; SQLite
    // does not enforce the foreign keys here, so clean up explicitly.
    let pool = state.storage.pool();
    sqlx::query(
        "DELETE FROM run_events WHERE run_id IN (SELECT id FROM runs WHERE conversation_id = ?)",
    )
    .bind(&id)
    .execute(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    sqlx::query("DELETE FROM runs WHERE conversation_id = ?")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    sqlx::query("DELETE FROM messages WHERE conversation_id = ?")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(&id)
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(())
}

#[tauri::command]
pub async fn rename_conversation(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("title must not be empty".to_string());
    }
    sqlx::query("UPDATE conversations SET data = json_set(data, '$.title', ?) WHERE id = ?")
        .bind(title)
        .bind(&id)
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(())
}

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
) -> Result<Vec<MessageView>, String> {
    fetch_messages(
        state.storage.pool(),
        &conversation_id,
        limit.unwrap_or(1000),
    )
    .await
}

/// Query helper kept separate from the command for integration tests.
pub async fn fetch_messages(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    conversation_id: &str,
    limit: u32,
) -> Result<Vec<MessageView>, String> {
    let rows: Vec<MessageRow> = sqlx::query_as(
        "SELECT id, \
                json_extract(data, '$.role') AS role, \
                json_extract(data, '$.content') AS content, \
                json_extract(data, '$.reasoning') AS reasoning, \
                json_extract(data, '$.modelId') AS model_id, \
                json_extract(data, '$.runId') AS run_id, \
                json_extract(data, '$.kind') AS kind, \
                created_at, \
                json_extract(data, '$.attachments') AS attachments \
         FROM messages WHERE conversation_id = ? \
         ORDER BY created_at DESC, rowid DESC LIMIT ?",
    )
    .bind(conversation_id)
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
            |(id, role, content, reasoning, model_id, run_id, kind, created_at, attachments)| {
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
                }
            },
        )
        .collect())
}

type MessageRow = (
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

fn normalize_ts(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Utc).to_rfc3339())
        .unwrap_or_else(|_| value.to_string())
}

fn parse_attachments(raw: Option<&str>) -> Vec<MessageAttachmentView> {
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
    let title: Option<String> =
        sqlx::query_scalar("SELECT json_extract(data, '$.title') FROM conversations WHERE id = ?")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .map_err(crate::error::cmd_err)?;
    let rows = sqlx::query(
        "SELECT json_extract(data, '$.role') AS role, json_extract(data, '$.content') AS content \
         FROM messages WHERE conversation_id = ? ORDER BY created_at ASC",
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
