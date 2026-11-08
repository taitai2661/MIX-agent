use super::*;

#[derive(Deserialize)]
pub struct ConversationCreate {
    pub mode: String,
    pub model_id: String,
    pub title: Option<String>,
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}

pub async fn project_exists(pool: &sqlx::SqlitePool, id: &str) -> Result<bool, String> {
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
    // `updated_at` is a generated column over this exact string. Returning the
    // same text as the cursor keeps the first and second page on one format.
    let updated_at_raw = now.to_rfc3339();
    let data = serde_json::json!({
        "title": title,
        "mode": input.mode,
        "modelId": input.model_id,
        "workspaceId": workspace_id.clone(),
        "projectId": project_id,
        "updatedAt": updated_at_raw,
        "state": "active",
    });
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
    state
        .spotlight
        .schedule(SpotlightKind::Conversation, &id)
        .await;
    Ok(ConversationSummary {
        id,
        title,
        mode: input.mode,
        model_id: input.model_id,
        updated_at: now,
        cursor: updated_at_raw,
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
    state
        .spotlight
        .schedule(SpotlightKind::Conversation, &conversation_id)
        .await;
    Ok(())
}

/// The folder a Run uses when the conversation is attached to no project.
///
/// Falling back to `std::env::current_dir()` was unsafe on desktop: a
/// Dock-launched macOS app runs with `/` as its CWD, so `files` (read: allow)
/// and `terminal` inherited the entire filesystem as their workspace. Every
/// projectless conversation now gets a private, app-owned scratch directory
/// instead, and the Run's system prompt names it so the model does not assume
/// the user's repository is the CWD.
pub(crate) fn projectless_scratch_root(
    state: &AppState,
    conversation_id: &str,
) -> Result<String, String> {
    let id = Uuid::parse_str(conversation_id)
        .map_err(|_| "invalid conversation_id".to_string())?
        .to_string();
    let dir = state.storage.data_dir().join("scratch").join(&id);
    std::fs::create_dir_all(&dir).map_err(|err| {
        format!(
            "could not prepare the projectless workspace at {}: {err}",
            dir.display()
        )
    })?;
    Ok(dir.to_string_lossy().into_owned())
}

/// Replace the "no project" context with an explicit scratch-workspace note.
pub(crate) fn projectless_workspace_note(root: &str) -> String {
    format!(
        "Workspace: no project folder is selected. All relative paths and the \
         terminal start in the private scratch folder `{root}`. Treat it as \
         temporary; files there are not part of the user's repository. If the \
         user asks to work on a real repository, ask them to select a project \
         first."
    )
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
    // A projectless conversation owned a scratch folder; deleting the thread
    // should not leave that behind. Best-effort: a locked file must not make
    // the delete command fail after the rows are already gone.
    if let Ok(uuid) = Uuid::parse_str(&id) {
        let scratch = state
            .storage
            .data_dir()
            .join("scratch")
            .join(uuid.to_string());
        let _ = tokio::fs::remove_dir_all(scratch).await;
    }
    state
        .spotlight
        .schedule_deleted(SpotlightKind::Conversation, &id)
        .await;
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
    state
        .spotlight
        .schedule(SpotlightKind::Conversation, &id)
        .await;
    Ok(())
}

/// Duplicate a conversation up to and including `message_id`.
///
/// Runs are intentionally not copied: a fork starts a new branch, so the
/// assistant rows lose their `runId` (the original run stays attached to the
/// original thread). Model / mode / project / workspace are preserved, which
/// is what makes a fork immediately usable.
#[tauri::command]
pub async fn fork_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
    message_id: String,
) -> Result<String, String> {
    let forked =
        fork_conversation_with(state.storage.pool(), &conversation_id, &message_id).await?;
    state
        .spotlight
        .schedule(SpotlightKind::Conversation, &forked)
        .await;
    Ok(forked)
}

pub async fn fork_conversation_with(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    conversation_id: &str,
    message_id: &str,
) -> Result<String, String> {
    let target_rowid: Option<i64> =
        sqlx::query_scalar("SELECT rowid FROM messages WHERE id = ? AND conversation_id = ?")
            .bind(message_id)
            .bind(conversation_id)
            .fetch_optional(pool)
            .await
            .map_err(crate::error::cmd_err)?;
    let Some(target_rowid) = target_rowid else {
        return Err(format!(
            "message {message_id} does not belong to conversation {conversation_id}"
        ));
    };

    let (owner_id, raw): (String, String) =
        sqlx::query_as("SELECT owner_id, data FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .fetch_optional(pool)
            .await
            .map_err(crate::error::cmd_err)?
            .ok_or_else(|| format!("conversation not found: {conversation_id}"))?;
    let mut data: serde_json::Value =
        serde_json::from_str(&raw).map_err(|err| format!("invalid conversation data: {err}"))?;
    let title = data
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("New chat");
    let now = Utc::now().to_rfc3339();
    data["title"] = serde_json::json!(format!("{title} (fork)"));
    data["createdAt"] = serde_json::json!(now);
    data["updatedAt"] = serde_json::json!(now);
    data["forkedFromConversationId"] = serde_json::json!(conversation_id);

    let messages: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, data, created_at FROM messages \
         WHERE conversation_id = ? AND rowid <= ? ORDER BY rowid",
    )
    .bind(conversation_id)
    .bind(target_rowid)
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;

    let new_id = Uuid::new_v4().to_string();
    let mut tx = pool.begin().await.map_err(crate::error::cmd_err)?;
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&new_id)
        .bind(&owner_id)
        .bind(data.to_string())
        .execute(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
    for (old_id, raw, created_at) in messages {
        let mut message: serde_json::Value =
            serde_json::from_str(&raw).map_err(|err| format!("invalid message data: {err}"))?;
        // A fork is a new branch: the original run's events and approvals stay
        // with the original conversation. Clearing the id avoids an assistant
        // bubble pointing at a run that is not in this thread.
        if message.get("runId").is_some() {
            message["runId"] = serde_json::Value::Null;
        }
        message["forkedFromMessageId"] = serde_json::json!(old_id);
        sqlx::query(
            "INSERT INTO messages (id, owner_id, conversation_id, data, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&owner_id)
        .bind(&new_id)
        .bind(message.to_string())
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .map_err(crate::error::cmd_err)?;
    }
    tx.commit().await.map_err(crate::error::cmd_err)?;
    Ok(new_id)
}
