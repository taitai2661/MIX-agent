use std::sync::Arc;

use crate::AppState;
use base64::Engine as _;
use chrono::Utc;
use mixd_core::runs::engine::{HistoryMessage, RunRequest};
use mixd_core::runs::state::RunId;
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
    /// Default `list_messages` page size.
    pub const DEFAULT_MESSAGE_LIMIT: u32 = 1000;
    /// Hard ceiling for `list_messages` pages.
    pub const MAX_MESSAGE_LIMIT: u32 = 500;
    /// Largest accepted user message (chars) and attachment payloads.
    pub const MAX_CONTENT_CHARS: usize = 200_000;
    pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
    pub const MAX_ATTACHMENTS_TOTAL_BYTES: usize = 20 * 1024 * 1024;
    pub const MAX_ATTACHMENTS: usize = 10;
    /// Output tokens reserved when Auto estimates the required context.
    pub const AUTO_RESERVED_OUTPUT_TOKENS: u64 = 4096;

    /// Stable `request_key` for idempotency: `conv:<id>:<millis>:<run>`.
    pub fn request_key(conversation_id: &str, now_millis: i64, run_id: &str) -> String {
        format!("conv:{conversation_id}:{now_millis}:{run_id}")
    }
}

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
    let rows: Vec<ConversationRow> = sqlx::query_as(&format!(
        "SELECT c.id, json_extract(c.data, '$.title'), json_extract(c.data, '$.mode'), \
          json_extract(c.data, '$.modelId'), json_extract(c.data, '$.updatedAt'), \
          (SELECT substr(json_extract(m.data, '$.content'), 1, {}) FROM messages m \
            WHERE m.conversation_id = c.id \
              AND json_extract(m.data, '$.role') IN ('user', 'assistant') \
            ORDER BY m.created_at DESC LIMIT 1), \
          (SELECT count(*) FROM messages m WHERE m.conversation_id = c.id \
            AND json_extract(m.data, '$.role') IN ('user', 'assistant')), \
          json_extract(c.data, '$.projectId'), \
          json_extract(c.data, '$.workspaceId') \
          FROM conversations c ORDER BY json_extract(c.data, '$.updatedAt') DESC",
        limits::PREVIEW_CHARS,
    ))
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                title,
                mode,
                model_id,
                updated_at,
                preview,
                message_count,
                project_id,
                workspace_id,
            )| {
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
        .unwrap_or_else(|_| {
            // A corrupt timestamp must not masquerade as "just now": `Utc::now`
            // sorted the broken row above every healthy thread and hid the
            // corruption. Epoch sorts it last, where it is visible but harmless.
            tracing::warn!(timestamp = %value, "unparseable timestamp, falling back to epoch");
            chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap_or_else(Utc::now)
        })
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
    dispatch(state, input, None).await
}

/// The turn an edit branches from, identified the way the transcript orders
/// messages: by `created_at` with `rowid` breaking ties.
struct TruncateTail {
    created_at: String,
    rowid: i64,
}

/// Start a Run, optionally replacing an existing tail first.
///
/// The truncation, the new Run row and the user turn are written in a single
/// transaction, so an edit either replaces the old tail or leaves the
/// conversation untouched. The engine is spawned only after that commit, which
/// is what keeps a failure while preparing the Run from stranding a transcript
/// that has already been deleted.
/// Validate a `send_message` payload before any storage or provider work.
fn validate_send_input(input: &SendMessageInput) -> Result<(), String> {
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

async fn dispatch(
    state: &AppState,
    input: SendMessageInput,
    truncate: Option<TruncateTail>,
) -> Result<SendMessageOutput, String> {
    validate_send_input(&input)?;
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

    // Explicit memory intake (memory spec §6 明示的な記憶): a「覚えて：…」or
    //「…を忘れて」message is saved / deleted synchronously here, before the Run
    // starts — waiting for post-response extraction would be the whole point
    // of saying it out loud. A temporary turn skips it, matching the Docker
    // edition's `_handle_explicit_memory_requests` (no trace, no memory).
    if !input.temporary_mode {
        match mixd_core::memory::handle_explicit(&state.storage, &input.content).await {
            Ok(outcome) => {
                if let Some(id) = outcome.created {
                    // The row is durable now; embedding it in the background
                    // means a provider without an embeddings endpoint never
                    // delays the turn that asked for it (spec §5.2).
                    crate::spawn_memory_embed(state.storage.clone(), state.embeddings.clone(), id);
                }
                if !outcome.forgotten.is_empty() {
                    tracing::info!(
                        forgotten = outcome.forgotten.len(),
                        "explicit memory forget applied"
                    );
                }
            }
            // The turn matters more than the intake: a storage hiccup while
            // saving an explicit memory must not block the message itself.
            Err(err) => tracing::warn!(%err, "explicit memory intake failed"),
        }
    }

    // A mode passed by the caller (the composer's mode selector) overrides the
    // mode stored at conversation creation; without this the selector was
    // silently ignored for existing conversations.
    let mode_str = input
        .mode
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| stored_mode.clone());
    let mode: mixd_core::Mode = mode_str.parse().map_err(|err| {
        tracing::warn!(%err, "dispatch: rejecting unknown mode");
        "invalid mode".to_string()
    })?;
    if mode_str != stored_mode {
        mixd_storage::log_write(
            sqlx::query("UPDATE conversations SET data = json_set(data, '$.mode', ?) WHERE id = ?")
                .bind(&mode_str)
                .bind(&input.conversation_id)
                .execute(pool)
                .await,
            "persist conversation mode",
        );
    }
    let requested_model_id = input.model_id.clone().unwrap_or(stored_model);
    // Docker parity: `auto` resolves to the best candidate per request. The
    // resolved id serves the turn; `requested_model_id` is kept so feedback
    // and history know this was an Auto turn.
    let is_auto = requested_model_id == "auto" || requested_model_id.is_empty();
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
        mixd_storage::log_write(
            sqlx::query(
                "UPDATE conversations SET data = json_set(data, '$.workspaceId', ?) WHERE id = ?",
            )
            .bind(workspace_id.to_string())
            .bind(&input.conversation_id)
            .execute(pool)
            .await,
            "persist conversation workspace",
        );
    }

    // Project context: instructions / sources / AGENTS.md → system prompt,
    // and the project folder → tool workspace root. Read-only and independent
    // of the new turn, so resolve it before opening the write transaction.
    let project = super::projects::load_project_runtime(pool, &input.conversation_id).await?;

    let tools = {
        let all = build_tool_specs(state, mode).await;
        if !input.temporary_mode {
            all
        } else if !input.allow_tools {
            // A temporary turn is private by default: no tool may leave a trace
            // on disk, the network or Memory.
            Arc::new(Vec::new())
        } else {
            // Even with tools allowed, Memory and Skill tools are excluded: they
            // exist to read and write durable state, which is what the mode is
            // opting out of.
            Arc::new(
                all.iter()
                    .filter(|spec| spec.name != "memory" && spec.name != "skill")
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        }
    };
    let reasoning = match input.reasoning.as_deref() {
        Some("low") => Some(ReasoningControl::Low),
        Some("medium") => Some(ReasoningControl::Medium),
        Some("high") => Some(ReasoningControl::High),
        Some("xhigh") => Some(ReasoningControl::Xhigh),
        _ => None,
    };

    let run_id = RunId::new_v4();
    let now = Utc::now();
    // Persist the mode's full budget (including the token ceiling) rather
    // than a hand-written subset: a missing `tokenLimit` silently falls back
    // to the load-time base, which downgrades agent runs when the base is the
    // 12-step default instead of the mode's own budget.
    let budget = mixd_core::runs::budget::Budget::for_mode(mode).to_json();
    let request_key = limits::request_key(
        &input.conversation_id,
        now.timestamp_millis(),
        &run_id.to_string(),
    );

    // Resolve the serving model. `auto` picks the best candidate per request;
    // anything else is served as-is.
    let has_image = input
        .attachments
        .iter()
        .any(|a| a.mime_type.starts_with("image/") || a.kind == "image");
    let tools_required = mode == mixd_core::Mode::Agent && !tools.is_empty();
    let (model_id, provider_id, auto_selection): (
        String,
        String,
        Option<mixd_core::auto::AutoSelection>,
    ) = if is_auto {
        let prior_contents: Vec<String> = sqlx::query_scalar::<_, Option<String>>(
            "SELECT json_extract(data, '$.content') FROM messages \
              WHERE conversation_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(&input.conversation_id)
        .bind(limits::AUTO_CONTEXT_LIMIT)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect();
        let mut context_parts: Vec<String> = prior_contents.into_iter().rev().collect();
        context_parts.push(input.content.clone());
        let resolved = super::auto::resolve_auto_model(
            pool,
            &input.content,
            mode.as_str(),
            tools_required,
            has_image,
            &context_parts,
            limits::AUTO_RESERVED_OUTPUT_TOKENS,
            &request_key,
            None,
        )
        .await?;
        let provider = resolve_provider_id(state, &resolved.model_id).await?;
        debug_assert_eq!(provider, resolved.provider_id);
        (resolved.model_id, provider, Some(resolved.selection))
    } else {
        let provider_id = resolve_provider_id(state, &requested_model_id).await?;
        (requested_model_id.clone(), provider_id, None)
    };

    // The truncation, the Run row and the user turn share one transaction: an
    // edit either replaces the old tail with a new Run, or leaves the
    // conversation exactly as it was. Splitting them (truncate, commit, then
    // start the Run) left the transcript deleted with nothing to replace it
    // whenever Run creation failed.
    let mut tx = pool.begin().await.map_err(crate::error::cmd_err)?;
    if truncate.is_some() {
        // The engine appends assistant turns to a transcript this edit is about
        // to rewrite, so an in-flight Run must finish (or be cancelled) first.
        // Checking inside the transaction closes the window between the check
        // and the delete that would let a Run start against a deleted tail.
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
    }
    // `request_key` is UNIQUE: a retry that reuses a key must not create a
    // second Run. The key embeds a fresh `run_id`, so collisions are only
    // possible on explicit retry — check before the INSERT to fail loudly
    // instead of surfacing a bare UNIQUE-constraint error.
    let duplicate: Option<String> = sqlx::query_scalar("SELECT id FROM runs WHERE request_key = ?")
        .bind(&request_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
    if duplicate.is_some() {
        return Err("duplicate request (request_key already exists)".to_string());
    }
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
            // Which provider served this Run — the memory extraction job
            // (memory spec §6) later replays the call through the same one.
            "providerId": provider_id,
            "requestedModelId": requested_model_id,
            "autoSelection": auto_selection,
            "workspaceId": workspace_id.to_string(),
            "status": "queued",
            "stepCount": 0,
            "toolCallCount": 0,
            "startedAt": now.to_rfc3339(),
            "updatedAt": now.to_rfc3339(),
            "summary": "",
            "budget": budget,
            "temporaryMode": input.temporary_mode,
        })
        .to_string(),
    )
    .bind(&input.conversation_id)
    .bind(&request_key)
    .execute(&mut *tx)
    .await
    .map_err(crate::error::cmd_err)?;

    // Branch from the edited turn inside the same transaction. The delete and
    // the history read below run on one connection, so the uncommitted delete is
    // visible here; a rollback restores the tail intact.
    if let Some(tail) = truncate {
        truncate_messages_from(
            &mut tx,
            &input.conversation_id,
            &tail.created_at,
            tail.rowid,
        )
        .await?;
    }

    // Before the user turn is inserted. `load_history` selects every message,
    // so loading it afterwards included the row just written and the engine
    // appended the same text again — every turn was sent to the model twice.
    let history: Vec<HistoryMessage> = load_history(&mut *tx, &input.conversation_id).await?;

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
        .execute(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;

    // Touch the conversation so it sorts to the top.
    mixd_storage::log_write(
        sqlx::query(
            "UPDATE conversations SET data = json_set(data, '$.updatedAt', ?) WHERE id = ?",
        )
        .bind(now.to_rfc3339())
        .bind(&input.conversation_id)
        .execute(&mut *tx)
        .await,
        "touch conversation",
    );

    // Truncate + Run + user turn are now committed together; only past this
    // point may the engine start and read the transcript.
    tx.commit().await.map_err(crate::error::cmd_err)?;

    // A temporary turn with no earlier messages created its conversation; once
    // the Run ends there is nothing to return to, so the thread goes too.
    let temporary_mode = input.temporary_mode;
    let purge_conversation = temporary_mode && history.is_empty();

    let request = RunRequest {
        run_id,
        conversation_id: Uuid::parse_str(&input.conversation_id).map_err(crate::error::cmd_err)?,
        workspace_id,
        root_path: project.root_path,
        additional_paths: project.additional_paths,
        mode,
        model_id: model_id.clone(),
        provider_id: provider_id.clone(),
        system: project.system,
        verify_command: project.verify_command,
        format_command: project.format_command,
        context_window: super::projects::context_window_for(pool, &model_id).await,
        temperature: input.temperature,
        reasoning,
        max_tokens: input.max_tokens,
        temporary: temporary_mode,
        tools,
        history,
        user_message: input.content,
        resume_from: None,
    };

    let engine = state.engine.clone();
    let pool = pool.clone();
    let conversation_id = input.conversation_id.clone();
    // Moved into the background task: stamping Auto provenance and learning
    // from the outcome both need the final status.
    let auto_stamp = auto_selection.clone();
    let auto_model_id = model_id.clone();
    let auto_provider_id = provider_id.clone();
    let auto_was_requested = is_auto;
    tokio::spawn(async move {
        // Run the engine in a child task so a panic there is observable as a
        // JoinError instead of silently aborting this task and leaving the
        // run row `running` forever.
        let engine_for_task = engine.clone();
        let child = tokio::spawn(async move { engine_for_task.execute(request).await });
        let result = child.await;
        // Stamp Auto provenance onto this run's assistant turns so feedback
        // can attribute the rating to the resolved model + route profile.
        // The engine owns transcript interleaving, so this happens after it
        // finishes rather than during.
        if let Some(selection) = &auto_stamp {
            let selection_json =
                serde_json::to_string(selection).unwrap_or_else(|_| "{}".to_string());
            mixd_storage::log_write(
                sqlx::query(
                    "UPDATE messages SET data = json_set(data, '$.autoSelection', json(?)) \
                     WHERE conversation_id = ? AND json_extract(data, '$.runId') = ? \
                     AND json_extract(data, '$.role') = 'assistant'",
                )
                .bind(&selection_json)
                .bind(&conversation_id)
                .bind(run_id.to_string())
                .execute(&pool)
                .await,
                "stamp auto selection",
            );
        }
        // Learn availability: one lightweight event per Auto turn keeps future
        // routing away from models/providers that just failed.
        if auto_was_requested {
            let (outcome, classification) = match &result {
                Ok(Ok(_)) => ("success", None),
                Ok(Err(err)) => {
                    let msg = err.to_string().to_lowercase();
                    let class = if msg.contains("401") || msg.contains("403") {
                        Some("auth")
                    } else if msg.contains("429") {
                        Some("rate_limit")
                    } else if msg.contains("timeout") || msg.contains("timed out") {
                        Some("timeout")
                    } else {
                        Some("other")
                    };
                    ("failure", class)
                }
                Err(_) => ("failure", Some("other")),
            };
            let profile = auto_stamp.as_ref().map(|s| s.profile.as_str());
            let required = auto_stamp.as_ref().map(|s| s.required_tokens);
            super::auto::record_auto_outcome(
                &pool,
                &auto_model_id,
                &auto_provider_id,
                mode.as_str(),
                outcome,
                classification,
                profile,
                required,
            )
            .await;
        }
        match result {
            Ok(Ok(outcome)) => {
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
            }
            Ok(Err(err)) => {
                tracing::error!(?err, %run_id, "run failed");
                record_run_failed(&pool, &engine, run_id, &err.to_string()).await;
            }
            Err(join_err) => {
                tracing::error!(?join_err, %run_id, "run task panicked or was cancelled");
                record_run_failed(
                    &pool,
                    &engine,
                    run_id,
                    &format!("run task failed: {join_err}"),
                )
                .await;
            }
        }

        if temporary_mode {
            if let Err(err) = purge_temporary_run(
                &pool,
                &conversation_id,
                &run_id.to_string(),
                purge_conversation,
            )
            .await
            {
                tracing::warn!(?err, "failed to purge a temporary run");
            }
        }
    });

    Ok(SendMessageOutput {
        run_id: run_id.to_string(),
    })
}

/// Persist a failed Run and emit its terminal events.
///
/// Shared by the `Err` and `JoinError` arms of the run-task join: the two used
/// to spell the same `UPDATE runs SET status = 'failed' …` plus `error` +
/// `status` emits inline, and only the log label differed.
async fn record_run_failed(
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
    dispatch(
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
    .await
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
    .map_err(crate::error::cmd_err)
}

async fn load_history<'e, E>(
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

async fn build_tool_specs(state: &AppState, mode: mixd_core::Mode) -> Arc<Vec<ToolSpec>> {
    // Docker parity: chat runs carry tools too (read/search/file/browser/MCP),
    // minus delegation (`agent`) and long-work planning (`todo`, `user_plan`).
    // Thinking additionally keeps the checklist; agent keeps everything.
    // Shared by the run's turns rather than rebuilt per turn.
    // Typed as `Mode` (not `&str`): the parse boundary in `dispatch` rejects
    // unknown modes with a warning instead of silently degrading the tool set.
    let specs = state.engine.tool_specs_for_mode(mode).await;
    Arc::new(specs)
}

#[tauri::command]
pub async fn delete_conversation(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut tx = state
        .storage
        .pool()
        .begin()
        .await
        .map_err(crate::error::cmd_err)?;

    // Refuse while a run is live: the engine holds `active` rows and keeps
    // writing messages/runs for a conversation that would no longer exist.
    // No `'paused'` in this list: `RunStatus` has no Paused variant, and a
    // status that can never be written must not gate the delete.
    let live: Option<String> = sqlx::query_scalar(
        "SELECT status FROM runs WHERE conversation_id = ? \
         AND status IN ('queued','running','waiting_approval','budget_extension_pending') \
         LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(crate::error::cmd_err)?;
    if let Some(status) = live {
        return Err(format!(
            "the conversation has a run in state `{status}`; stop it before deleting"
        ));
    }

    // `runs`, `tool_calls`, `approvals`, `run_events`, `run_checkpoints` and
    // `message_feedback` all carry foreign keys into this conversation's rows
    // and `PRAGMA foreign_keys` is ON, so each has to go leaf-first. The old
    // comment claiming SQLite did not enforce them was wrong: deleting any
    // conversation that had ever made a tool call failed outright, and it did
    // so across four separate statements with no transaction — so a failure
    // part-way left `run_events` deleted while `runs` survived.
    let run_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM runs WHERE conversation_id = ?")
        .bind(&id)
        .fetch_all(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
    for run_id in &run_ids {
        delete_run_cascade(&mut tx, run_id).await?;
    }

    // `message_feedback.message_id` → `messages.id`: feedback first.
    sqlx::query(
        "DELETE FROM message_feedback WHERE message_id IN \
         (SELECT id FROM messages WHERE conversation_id = ?)",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(crate::error::cmd_err)?;

    // `messages.conversation_id` → `conversations.id`: messages before the
    // conversation itself, all inside the one transaction.
    sqlx::query("DELETE FROM messages WHERE conversation_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;

    let result = sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
    if result.rows_affected() == 0 {
        return Err(format!("conversation not found: {id}"));
    }

    tx.commit().await.map_err(crate::error::cmd_err)?;
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
    let result =
        sqlx::query("UPDATE conversations SET data = json_set(data, '$.title', ?) WHERE id = ?")
            .bind(title)
            .bind(&id)
            .execute(state.storage.pool())
            .await
            .map_err(crate::error::cmd_err)?;
    if result.rows_affected() == 0 {
        return Err(format!("conversation not found: {id}"));
    }
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
    // Clamp the page size: an unbounded LIMIT lets a caller dump the whole
    // table in one IPC round trip.
    let limit = limit
        .unwrap_or(limits::DEFAULT_MESSAGE_LIMIT)
        .clamp(1, limits::MAX_MESSAGE_LIMIT);
    fetch_messages(state.storage.pool(), &conversation_id, limit).await
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
    let limit = limit.clamp(1, limits::MAX_MESSAGE_LIMIT);
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
           AND json_extract(data, '$.role') IN ('user', 'assistant') \
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
