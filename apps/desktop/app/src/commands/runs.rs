use std::sync::Arc;

use crate::AppState;
use mixd_core::runs::state::RunView;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

/// Append a message to an active Run's next turn.
///
/// Returns `false` when the Run has already finished, so the UI can send the
/// text as a normal new turn instead of losing it in a queue nobody drains.
#[tauri::command]
pub async fn steer_run(
    state: State<'_, AppState>,
    run_id: String,
    content: String,
) -> Result<bool, String> {
    if content.trim().is_empty() {
        return Err("message is empty".to_string());
    }
    let id = Uuid::parse_str(&run_id).map_err(crate::error::cmd_err)?;
    Ok(state.engine.steer(id, content).await)
}

#[derive(Serialize)]
pub struct SteeringView {
    /// Sequence assigned when the message was queued; stable for edit/remove.
    pub seq: u64,
    pub content: String,
}

/// Messages queued for the next turn of an active Run.
///
/// The queue lives in memory and is drained at the next model turn, so the UI
/// polls while a Run is busy rather than treating this as durable history.
#[tauri::command]
pub async fn list_steering(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Vec<SteeringView>, String> {
    let id = Uuid::parse_str(&run_id).map_err(crate::error::cmd_err)?;
    Ok(state
        .engine
        .steering_queue(id)
        .await
        .into_iter()
        .map(|entry| SteeringView {
            seq: entry.seq,
            content: entry.text,
        })
        .collect())
}

/// Rewrite a queued steering message before it is folded into the next turn.
#[tauri::command]
pub async fn update_steering(
    state: State<'_, AppState>,
    run_id: String,
    seq: u64,
    content: String,
) -> Result<bool, String> {
    if content.trim().is_empty() {
        return Err("message is empty".to_string());
    }
    let id = Uuid::parse_str(&run_id).map_err(crate::error::cmd_err)?;
    Ok(state.engine.update_steering(id, seq, content).await)
}

/// Remove a queued steering message before it is folded into the next turn.
#[tauri::command]
pub async fn remove_steering(
    state: State<'_, AppState>,
    run_id: String,
    seq: u64,
) -> Result<bool, String> {
    let id = Uuid::parse_str(&run_id).map_err(crate::error::cmd_err)?;
    Ok(state.engine.remove_steering(id, seq).await)
}

/// Restore every file the Run changed through `edit` / `files` to its
/// pre-Run content. Terminal commands are out of scope.
#[tauri::command]
pub async fn revert_run(state: State<'_, AppState>, run_id: String) -> Result<usize, String> {
    let id = Uuid::parse_str(&run_id).map_err(crate::error::cmd_err)?;
    state
        .engine
        .revert_run(id)
        .await
        .map_err(crate::error::cmd_err)
}

/// What a Git-backed rollback would touch for this Run.
///
/// `available: false` means the workspace is not a Git repository or the Run
/// predates baseline capture; the activity panel then keeps the old
/// edit/files-only wording.
#[tauri::command]
pub async fn run_workspace_changes(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<crate::workspace_baseline::WorkspaceChanges, String> {
    let id = crate::workspace_baseline::parse_run_id(&run_id)?;
    crate::workspace_baseline::changes(state.storage.pool(), id).await
}

/// Revert the Run through the Git baseline first, then the engine's in-memory
/// edit/files snapshots.
///
/// The Git pass is what makes terminal-created files and renames recoverable.
/// It restores the whole worktree to the pre-Run snapshot, so the UI must warn
/// that edits made after the Run are lost as well.
#[tauri::command]
pub async fn revert_run_workspace(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<usize, String> {
    let id = crate::workspace_baseline::parse_run_id(&run_id)?;
    let live: Option<String> = sqlx::query_scalar(
        "SELECT status FROM runs WHERE id = ? AND status IN \
         ('queued','running','waiting_approval','budget_extension_pending')",
    )
    .bind(id.to_string())
    .fetch_optional(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    if live.is_some() {
        return Err("実行中のRunは取り消せません。先に停止してください。".to_string());
    }
    let git_touched = crate::workspace_baseline::revert(state.storage.pool(), id).await?;
    let file_touched = state
        .engine
        .revert_run(id)
        .await
        .map_err(crate::error::cmd_err)?;
    Ok(git_touched + file_touched)
}

/// Revert only the selected paths. The whole-run baseline stays available.
#[tauri::command]
pub async fn revert_run_workspace_paths(
    state: State<'_, AppState>,
    run_id: String,
    paths: Vec<String>,
) -> Result<usize, String> {
    let id = crate::workspace_baseline::parse_run_id(&run_id)?;
    if paths.is_empty() {
        return Ok(0);
    }
    let live: Option<String> = sqlx::query_scalar(
        "SELECT status FROM runs WHERE id = ? AND status IN \
         ('queued','running','waiting_approval','budget_extension_pending')",
    )
    .bind(id.to_string())
    .fetch_optional(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    if live.is_some() {
        return Err("実行中のRunは取り消せません。先に停止してください。".to_string());
    }
    crate::workspace_baseline::revert_paths(state.storage.pool(), id, &paths).await
}

#[tauri::command]
pub async fn get_run(state: State<'_, AppState>, id: String) -> Result<RunView, String> {
    let run_id = Uuid::parse_str(&id).map_err(crate::error::cmd_err)?;
    if let Some(view) = state.engine.run_state(run_id).await {
        return Ok(view);
    }
    // The engine reads the `runs` table, so `None` means missing or corrupt —
    // probe the row directly to tell the two apart instead of a bare 404.
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM runs WHERE id = ?")
        .bind(run_id.to_string())
        .fetch_optional(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    match exists {
        Some(_) => Err(format!("run data is corrupt, cannot display: {id}")),
        None => Err("run not found".to_string()),
    }
}

#[tauri::command]
pub async fn list_runs(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<RunView>, String> {
    let conversation_id = Uuid::parse_str(&conversation_id).map_err(crate::error::cmd_err)?;
    // `engine.list_runs` reads the persisted `runs` rows (not just live
    // in-memory runs), so restarted-app and finished runs are included.
    Ok(state.engine.list_runs(conversation_id).await)
}

/// A Run left mid-flight by a crash, force-quit or the quit guard.
///
/// Its status is `interrupted`; `checkpoint_id` is the newest snapshot the
/// user can resume from. The startup banner uses this instead of making the
/// user find the right thread and open Activity first.
#[derive(Debug, Clone, Serialize)]
pub struct InterruptedRunView {
    pub run_id: String,
    pub conversation_id: String,
    pub title: String,
    pub mode: String,
    pub updated_at: String,
    pub step_count: u32,
    pub checkpoint_id: Option<String>,
    pub checkpoint_step: Option<u32>,
}

#[tauri::command]
pub async fn list_interrupted_runs(
    state: State<'_, AppState>,
) -> Result<Vec<InterruptedRunView>, String> {
    type Row = (
        String,
        String,
        String,
        String,
        String,
        i64,
        Option<String>,
        Option<i64>,
    );
    let rows: Vec<Row> = sqlx::query_as::<_, Row>(
        "SELECT r.id, r.conversation_id, \
                COALESCE(json_extract(c.data, '$.title'), ''), \
                COALESCE(json_extract(r.data, '$.mode'), 'chat'), \
                COALESCE(json_extract(r.data, '$.updatedAt'), r.created_at), \
                COALESCE(json_extract(r.data, '$.stepCount'), 0), \
                (SELECT cp.id FROM run_checkpoints cp WHERE cp.run_id = r.id \
                  ORDER BY cp.step DESC, cp.created_at DESC LIMIT 1), \
                (SELECT cp.step FROM run_checkpoints cp WHERE cp.run_id = r.id \
                  ORDER BY cp.step DESC, cp.created_at DESC LIMIT 1) \
           FROM runs r JOIN conversations c ON c.id = r.conversation_id \
          WHERE r.status = 'interrupted' \
          ORDER BY COALESCE(json_extract(r.data, '$.updatedAt'), r.created_at) DESC \
          LIMIT 20",
    )
    .fetch_all(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(run_id, conversation_id, title, mode, updated_at, step, cp, cp_step)| {
                InterruptedRunView {
                    run_id,
                    conversation_id,
                    title,
                    mode,
                    updated_at,
                    step_count: step.max(0) as u32,
                    checkpoint_id: cp,
                    checkpoint_step: cp_step.map(|v| v.max(0) as u32),
                }
            },
        )
        .collect())
}

/// A Run currently queued, running or waiting for an answer.
#[derive(Debug, Clone, Serialize)]
pub struct ActiveRunView {
    pub run_id: String,
    pub conversation_id: String,
    pub title: String,
    pub mode: String,
    pub status: String,
    pub updated_at: String,
    pub step_count: u32,
}

#[tauri::command]
pub async fn list_active_runs(state: State<'_, AppState>) -> Result<Vec<ActiveRunView>, String> {
    type Row = (String, String, String, String, String, String, i64);
    let rows: Vec<Row> = sqlx::query_as::<_, Row>(
        "SELECT r.id, r.conversation_id, \
                COALESCE(json_extract(c.data, '$.title'), ''), \
                COALESCE(json_extract(r.data, '$.mode'), 'chat'), \
                r.status, \
                COALESCE(json_extract(r.data, '$.updatedAt'), r.created_at), \
                COALESCE(json_extract(r.data, '$.stepCount'), 0) \
           FROM runs r JOIN conversations c ON c.id = r.conversation_id \
          WHERE r.status IN ('queued','running','waiting_approval','budget_extension_pending') \
          ORDER BY COALESCE(json_extract(r.data, '$.updatedAt'), r.created_at) DESC \
          LIMIT 20",
    )
    .fetch_all(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(run_id, conversation_id, title, mode, status, updated_at, step)| ActiveRunView {
                run_id,
                conversation_id,
                title,
                mode,
                status,
                updated_at,
                step_count: step.max(0) as u32,
            },
        )
        .collect())
}

/// One persisted `run_events` row in the same shape the live `run_event`
/// Tauri event carries, so the transcript reducer can consume both.
#[derive(Debug, Clone, Serialize)]
pub struct RunEventView {
    pub run_id: String,
    pub sequence: i64,
    pub kind: String,
    pub data: serde_json::Value,
    pub created_at: String,
}

#[tauri::command]
pub async fn list_run_events(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Vec<RunEventView>, String> {
    // Buffered token deltas are not in the table yet; a reload that read the
    // log first would come back with a truncated transcript.
    state.engine.flush_all_events().await;
    fetch_run_events(state.storage.pool(), &run_id).await
}

/// Query helper kept separate from the command for integration tests.
pub async fn fetch_run_events(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    run_id: &str,
) -> Result<Vec<RunEventView>, String> {
    let limit = mixd_core::runs::events::MAX_REPLAY_EVENTS;
    let window = mixd_core::runs::events::load_replay_window(pool, run_id, limit)
        .await
        .map_err(crate::error::cmd_err)?;
    let omitted = window.omitted(limit);
    // `data` is a JSON string; a corrupt row is a warn + Null, never a silent
    // empty bubble — the caller cannot tell "no payload" from "unreadable".
    let mut events: Vec<RunEventView> = window
        .rows
        .into_iter()
        .map(|(run_id, sequence, kind, data, created_at)| {
            let data = serde_json::from_str(&data).unwrap_or_else(|err| {
                tracing::warn!(%run_id, sequence, %err, "run event has a corrupt data blob; showing Null");
                serde_json::Value::Null
            });
            RunEventView {
                run_id,
                sequence,
                kind,
                data,
                created_at,
            }
        })
        .collect();
    if omitted > 0 {
        // Never hide a cut transcript: the marker tells the reducer that the
        // middle of the log is missing, including how many rows were dropped.
        let sequence = events.last().map(|event| event.sequence + 1).unwrap_or(1);
        events.push(RunEventView {
            run_id: run_id.to_string(),
            sequence,
            kind: "replay_truncated".to_string(),
            data: serde_json::json!({ "limit": limit, "omitted": omitted }),
            created_at: chrono::Utc::now().to_rfc3339(),
        });
    }
    Ok(events)
}

#[tauri::command]
pub async fn cancel_run(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let run_id = Uuid::parse_str(&id).map_err(crate::error::cmd_err)?;
    // A caller cancelling a nonexistent id must hear about it: the old code
    // always returned `Ok`, so a typo looked like a successful cancel.
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM runs WHERE id = ?")
        .bind(run_id.to_string())
        .fetch_optional(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    if exists.is_none() {
        return Err(format!("run not found: {id}"));
    }
    state.engine.cancel(run_id).await;
    let now = chrono::Utc::now().to_rfc3339();
    // `run_view_from_row` reads the status out of `data`, so updating only the
    // column left cancelled runs showing `waiting_approval` in the list.
    // Keep both in step, the same way `quit_app` does.
    let result = sqlx::query(
        "UPDATE runs SET status = 'cancelled', \
         data = json_set(data, '$.status', 'cancelled', '$.updatedAt', ?1) \
         WHERE id = ?2 AND status NOT IN ('completed', 'failed', 'cancelled', 'interrupted')",
    )
    .bind(&now)
    .bind(run_id.to_string())
    .execute(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    // Only emit the cancelled transition when the row was actually flipped.
    // Emitting unconditionally meant a nonexistent or already-terminal run
    // still broadcast a "cancelled" event the UI would render.
    if result.rows_affected() > 0 {
        state
            .engine
            .emit(
                run_id,
                "status",
                serde_json::json!({ "status": "cancelled" }),
            )
            .await;
    }
    Ok(())
}

/// Stop every live Run and quit.
///
/// Called only after the WebView confirmed that losing the in-flight work is
/// intentional. The rows are moved to `interrupted` *before* the process exits
/// so the next launch can offer a checkpoint resume instead of showing a Run
/// that is still "running" forever.
#[tauri::command]
pub async fn quit_app(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let run_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM runs WHERE status IN \
         ('queued','running','waiting_approval','budget_extension_pending')",
    )
    .fetch_all(state.storage.pool())
    .await
    .unwrap_or_default();
    for raw in &run_ids {
        if let Ok(id) = Uuid::parse_str(raw) {
            state.engine.cancel(id).await;
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    let _ = sqlx::query(
        "UPDATE runs SET status = 'interrupted', \
         data = json_set(data, '$.status', 'interrupted', '$.updatedAt', ?1) \
         WHERE status IN ('queued','running','waiting_approval','budget_extension_pending')",
    )
    .bind(&now)
    .execute(state.storage.pool())
    .await;
    let _ = sqlx::query("UPDATE approvals SET status = 'expired' WHERE status = 'pending'")
        .execute(state.storage.pool())
        .await;
    crate::allow_exit();
    app.exit(0);
    Ok(())
}

#[derive(Deserialize)]
pub struct ResumeInput {
    pub id: String,
    pub from_checkpoint: Option<String>,
    pub unknown_actions: Vec<UnknownAction>,
}

#[derive(Deserialize, Clone, Serialize)]
pub struct UnknownAction {
    pub tool_call_id: String,
    /// `"retry" | "discard" | "mark_unknown"`
    pub action: String,
}

/// The columns `resume_run` reads back out of the `runs` row: everything the
/// engine needs to rebuild the original request, in select order.
type ResumeRunRow = (String, String, String, Option<String>, Option<bool>);

#[tauri::command]
pub async fn resume_run(state: State<'_, AppState>, input: ResumeInput) -> Result<String, String> {
    let run_id = Uuid::parse_str(&input.id).map_err(crate::error::cmd_err)?;
    let checkpoint_id = input
        .from_checkpoint
        .clone()
        .ok_or("from_checkpoint is required")?;
    let snapshot = state
        .engine
        .checkpoints
        .load(run_id, &checkpoint_id, state.storage.as_ref())
        .await
        .ok_or("checkpoint not found")?;

    // Apply the `unknown_actions` policy to Tool calls that were left in
    // `executing` before the interruption. Reject unknown action strings
    // instead of silently mapping them to "unknown", and reject ids that do
    // not belong to this run instead of silently ignoring them.
    for action in &input.unknown_actions {
        let status = match action.action.as_str() {
            "retry" => "pending",
            "discard" => "discarded",
            "mark_unknown" => "unknown",
            other => return Err(format!("unknown action: {other}")),
        };
        let result = sqlx::query("UPDATE tool_calls SET status = ? WHERE id = ? AND run_id = ?")
            .bind(status)
            .bind(&action.tool_call_id)
            .bind(run_id.to_string())
            .execute(state.storage.pool())
            .await
            .map_err(crate::error::cmd_err)?;
        if result.rows_affected() == 0 {
            return Err(format!("unknown tool_call: {}", action.tool_call_id));
        }
    }

    // Load the Run's original request parameters so the loop can restart.
    let row: Option<ResumeRunRow> = sqlx::query_as::<_, ResumeRunRow>(
        "SELECT conversation_id, json_extract(data, '$.modelId'), \
             json_extract(data, '$.mode'), json_extract(data, '$.workspaceId'), \
             json_extract(data, '$.temporaryMode') \
             FROM runs WHERE id = ?",
    )
    .bind(run_id.to_string())
    .fetch_optional(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    let (conversation_id, model_id, mode_str, workspace_id, temporary) =
        row.ok_or("run not found")?;
    let temporary = temporary.unwrap_or(false);
    let mode: mixd_core::Mode = mode_str.parse().map_err(|err| {
        tracing::warn!(%err, "resume: rejecting unknown mode");
        "invalid mode".to_string()
    })?;

    // Claim the run atomically: the conditional UPDATE flips the row to
    // `running` only when it is still resumable, so two concurrent resumes
    // cannot both spawn an engine loop for the same run_id. A separate
    // SELECT-then-UPDATE left that window open.
    // (Status check happens at claim time below, after the request is built,
    // so a provider/model failure never leaves the row flipped to `running`.)
    // Resolved from the run's own `modelId`, which is `<provider_id>:<model>`.
    // This used to be `providers.list().first().unwrap_or_default()`, i.e. an
    // empty string whenever the registry was empty: the request was built with
    // `provider_id: ""` and only failed later inside the Engine, after the row
    // had already been flipped to `running`. A run whose provider was deleted
    // meanwhile also silently resumed on whichever provider sorted first by
    // label.
    let provider_id = crate::resolve_provider_for_model(
        &state.engine.providers,
        state.storage.as_ref(),
        state.secrets.as_ref(),
        &model_id,
    )
    .await
    .map_err(crate::error::cmd_err)?;
    let tools = Arc::new(state.engine.tool_specs_for_mode(mode).await);

    // Re-apply the project context so a resumed run keeps its folder scope and
    // instructions even if the original request predates the project feature.
    let mut project =
        crate::commands::projects::load_project_runtime(state.storage.pool(), &conversation_id)
            .await?;
    // A project deleted between the interruption and the resume must not send
    // the Run back to the process CWD (`/` when launched from the Dock). Reuse
    // the same scratch folder the original turn was given.
    if project.root_path.is_none() {
        let scratch = crate::commands::conversations::projectless_scratch_root(
            state.inner(),
            &conversation_id,
        )?;
        project.system = Some(match project.system.take() {
            Some(mut existing) => {
                existing.push_str("\n\n");
                existing.push_str(&crate::commands::conversations::projectless_workspace_note(
                    &scratch,
                ));
                existing
            }
            None => crate::commands::conversations::projectless_workspace_note(&scratch),
        });
        project.root_path = Some(scratch);
    }

    // A resumed Run that never had a baseline (it predates the feature, or the
    // original workspace was not a Git repo) gets one now, so the part that
    // this resume changes is still revertible.
    crate::workspace_baseline::capture_and_store(
        state.storage.pool(),
        project.root_path.as_deref(),
        run_id,
    )
    .await;

    let context_window =
        crate::commands::projects::context_window_for(state.storage.pool(), &model_id).await;
    let locality =
        crate::commands::providers::provider_locality(state.storage.pool(), &provider_id).await;
    let settings = crate::commands::settings::read_settings(state.storage.pool())
        .await
        .ok();
    let context_window = crate::commands::projects::effective_context_window(
        context_window,
        locality.is_managed,
        settings
            .as_ref()
            .and_then(|value| value.local_context_window),
    );
    let keep_alive = if locality.is_managed && locality.is_ollama {
        settings.and_then(|value| value.local_keep_alive)
    } else {
        None
    };

    let request = mixd_core::runs::engine::RunRequest {
        run_id,
        conversation_id: Uuid::parse_str(&conversation_id).map_err(crate::error::cmd_err)?,
        // Reuse the Run's original workspace so its browser profile,
        // workspace-scoped permission rules and terminal session stay the same
        // across the interruption. Only a pre-workspace Run mints a new one.
        workspace_id: workspace_id
            .and_then(|w| Uuid::parse_str(&w).ok())
            .unwrap_or_else(Uuid::new_v4),
        root_path: project.root_path,
        additional_paths: project.additional_paths,
        mode,
        model_id,
        provider_id,
        system: project.system,
        verify_command: project.verify_command,
        format_command: project.format_command,
        context_window,
        temperature: None,
        reasoning: None,
        max_tokens: None,
        keep_alive,
        temporary,
        tools,
        history: Vec::new(),
        user_message: String::new(),
        resume_from: Some(snapshot),
        auto_fallbacks: Vec::new(),
        step_models: Vec::new(),
        monthly_pressure: 0.0,
    };

    // Keep the `data` blob in sync with the status column (the UI and
    // `run_view_from_row` read `$.status` / `$.updatedAt` out of it).
    let claimed = sqlx::query(
        "UPDATE runs SET status = 'running', \
          data = json_set(data, '$.status', 'running', '$.updatedAt', ?) \
          WHERE id = ? AND status IN ('interrupted', 'failed', 'cancelled')",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(run_id.to_string())
    .execute(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    if claimed.rows_affected() == 0 {
        let current: Option<String> = sqlx::query_scalar("SELECT status FROM runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_optional(state.storage.pool())
            .await
            .map_err(crate::error::cmd_err)?;
        return match current.as_deref() {
            Some(other) => Err(format!(
                "run is {other}; only interrupted, failed or cancelled runs can be resumed"
            )),
            None => Err("run not found".to_string()),
        };
    }
    state
        .engine
        .emit(
            run_id,
            "status",
            serde_json::json!({ "status": "running", "resumed_from": checkpoint_id }),
        )
        .await;

    let engine = state.engine.clone();
    let pool = state.storage.pool().clone();
    tokio::spawn(async move {
        // Run in a child task so a panic is observable as a JoinError instead
        // of silently aborting this task and leaving the row `running`.
        let engine_for_task = engine.clone();
        let child = tokio::spawn(async move { engine_for_task.execute(request).await });
        let result = child.await;
        // Both arms matter: swallowing `Err` here left the row `running`
        // forever, and it is also counted as an active run by diagnostics.
        let status = match &result {
            Ok(Ok(outcome)) => outcome.status.as_str().to_string(),
            Ok(Err(err)) => {
                tracing::error!(?err, %run_id, "resumed run failed");
                "failed".to_string()
            }
            Err(join_err) => {
                tracing::error!(?join_err, %run_id, "resumed run task panicked or was cancelled");
                "failed".to_string()
            }
        };
        mixd_storage::log_write(
            sqlx::query("UPDATE runs SET status = ? WHERE id = ?")
                .bind(&status)
                .bind(run_id.to_string())
                .execute(&pool)
                .await,
            "record resumed run status",
        );
        if let Ok(Err(err)) = &result {
            let message = err.to_string();
            engine
                .emit(
                    run_id,
                    "error",
                    serde_json::json!({ "error": message.clone() }),
                )
                .await;
            engine
                .emit(
                    run_id,
                    "status",
                    serde_json::json!({ "status": "failed", "error": message }),
                )
                .await;
        } else if let Err(join_err) = &result {
            let message = format!("run task failed: {join_err}");
            engine
                .emit(
                    run_id,
                    "error",
                    serde_json::json!({ "error": message.clone() }),
                )
                .await;
            engine
                .emit(
                    run_id,
                    "status",
                    serde_json::json!({ "status": "failed", "error": message }),
                )
                .await;
        }
    });

    Ok(run_id.to_string())
}

#[derive(Deserialize)]
pub struct BudgetExtensionInput {
    pub id: String,
    pub grant: bool,
    pub step_limit: Option<u32>,
    pub tool_call_limit: Option<u32>,
    pub wall_clock_minutes: Option<u32>,
}

#[tauri::command]
pub async fn budget_extension(
    state: State<'_, AppState>,
    input: BudgetExtensionInput,
) -> Result<(), String> {
    apply_budget_extension(&state.engine, input).await
}

/// Validate and deliver a budget answer.
///
/// The engine only accepts an answer for a Run sitting in
/// `budget_extension_pending`, so this is where the WebView's request becomes
/// either a delivered decision or an error string. Split out from the command
/// so the rejection path can be covered without a Tauri handle.
pub async fn apply_budget_extension(
    engine: &mixd_core::runs::engine::RunEngine,
    input: BudgetExtensionInput,
) -> Result<(), String> {
    // Reject nonsensical limits before the engine sees them: a 0-minute
    // wall clock would instantly exhaust the budget, and a huge step limit
    // would silently drop the intended cap.
    if input.grant {
        if let Some(m) = input.wall_clock_minutes {
            if m == 0 || m > 1440 {
                return Err("wall_clock_minutes must be between 1 and 1440".to_string());
            }
        }
        if let Some(s) = input.step_limit {
            if s == 0 || s > 10_000 {
                return Err("step_limit must be between 1 and 10000".to_string());
            }
        }
        if let Some(t) = input.tool_call_limit {
            if t == 0 || t > 10_000 {
                return Err("tool_call_limit must be between 1 and 10000".to_string());
            }
        }
    }
    let run_id = Uuid::parse_str(&input.id).map_err(crate::error::cmd_err)?;
    let budget = input.grant.then(|| {
        serde_json::json!({
            "stepLimit": input.step_limit,
            "toolCallLimit": input.tool_call_limit,
            "wallClockMinutes": input.wall_clock_minutes,
        })
    });
    engine.resolve_budget(run_id, input.grant, budget).await?;
    engine
        .emit(
            run_id,
            "budget_extension_resolved",
            serde_json::json!({ "granted": input.grant }),
        )
        .await;
    Ok(())
}

#[tauri::command]
pub async fn replay_run_events(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    // A malformed id must be rejected here: passing it through lets the replay
    // query match nothing and silently succeed.
    Uuid::parse_str(&id).map_err(|_| format!("invalid run id: {id}"))?;
    crate::run_events::replay(&app, state.engine.as_ref(), &id).await;
    Ok(())
}

#[derive(Serialize)]
pub struct CheckpointView {
    pub id: String,
    pub step_index: u32,
    pub created_at: String,
}
#[tauri::command]
pub async fn list_checkpoints(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<CheckpointView>, String> {
    let run_id = Uuid::parse_str(&id).map_err(crate::error::cmd_err)?;
    let rows = state
        .engine
        .checkpoints
        .list(run_id, state.storage.as_ref())
        .await;
    Ok(rows
        .into_iter()
        .map(|(id, step_index, created_at)| CheckpointView {
            id,
            step_index,
            created_at,
        })
        .collect())
}

/// Export one Run's persisted event log as JSONL: one JSON object per line in
/// `RunEventView` shape (`run_id`, `sequence`, `kind`, `data`, `created_at`).
///
/// The frontend saves the returned string with the file picker
/// (`pick_save_file` + `save_text_file`), so this command never touches the
/// filesystem itself. The output is also the import format accepted by
/// [`import_run_events`], making a JSONL dump a portable Run archive.
#[tauri::command]
pub async fn export_run_events_jsonl(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<String, String> {
    // Reject a malformed id up front: the fetch would silently return an
    // empty log for a run that never existed.
    Uuid::parse_str(&run_id).map_err(|_| format!("invalid run id: {run_id}"))?;
    let events = fetch_run_events(state.storage.pool(), &run_id).await?;
    let mut out = String::new();
    for event in &events {
        let line = serde_json::to_string(event).map_err(crate::error::cmd_err)?;
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

/// Render one Run's event log as a human-readable Markdown transcript.
///
/// Tool calls keep their arguments and outcome, approvals/checkpoints/errors
/// become sections, and streamed text is concatenated in event order. This is
/// the "share this run" format; [`export_run_events_jsonl`] is the
/// machine-readable one.
#[tauri::command]
pub async fn export_run_markdown(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<String, String> {
    Uuid::parse_str(&run_id).map_err(|_| format!("invalid run id: {run_id}"))?;
    let events = fetch_run_events(state.storage.pool(), &run_id).await?;
    Ok(render_run_markdown(&run_id, &events))
}

pub fn render_run_markdown(run_id: &str, events: &[RunEventView]) -> String {
    fn short_json(value: &serde_json::Value) -> String {
        let text = serde_json::to_string_pretty(value).unwrap_or_default();
        const LIMIT: usize = 2000;
        if text.len() > LIMIT {
            format!("{}…", &text[..LIMIT])
        } else {
            text
        }
    }
    let mut out = format!("# Run {run_id}\n\n");
    let mut text = String::new();
    let mut reasoning = String::new();
    for event in events {
        let data = &event.data;
        let field = |key: &str| data.get(key).and_then(|v| v.as_str()).unwrap_or("");
        match event.kind.as_str() {
            "text" => text.push_str(field("delta")),
            "reasoning" => reasoning.push_str(field("delta")),
            "status" => {
                let status = field("status");
                if !status.is_empty() {
                    out.push_str(&format!("**status:** {status}\n\n"));
                }
                if let Some(err) = data.get("error").and_then(|v| v.as_str()) {
                    out.push_str(&format!("**error:** {err}\n\n"));
                }
            }
            "error" => {
                let err = field("error");
                if !err.is_empty() {
                    out.push_str(&format!("## Error\n\n{err}\n\n"));
                }
            }
            "tool_started" => {
                let name = field("name");
                let name = if name.is_empty() { "tool" } else { name };
                out.push_str(&format!("## Tool: {name}\n\n"));
                if let Some(args) = data.get("arguments") {
                    out.push_str(&format!("```json\n{}\n```\n\n", short_json(args)));
                }
            }
            "tool_result" => {
                if let Some(err) = data.get("error").and_then(|v| v.as_str()) {
                    out.push_str(&format!("**failed:** {err}\n\n"));
                } else if let Some(result) = data.get("result") {
                    out.push_str(&format!("```json\n{}\n```\n\n", short_json(result)));
                }
            }
            "tool_approval_requested" => {
                out.push_str(&format!("**approval requested:** {}\n\n", field("name")));
            }
            "checkpoint_saved" => {
                let id = data
                    .get("checkpoint_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                out.push_str(&format!("**checkpoint:** {id}\n\n"));
            }
            "stagnation_detected" => {
                let hint = field("hint");
                if !hint.is_empty() {
                    out.push_str(&format!("**stagnation:** {hint}\n\n"));
                }
            }
            "verification_failed" => {
                let reason = field("reason");
                if !reason.is_empty() {
                    out.push_str(&format!("**verification failed:** {reason}\n\n"));
                }
            }
            _ => {}
        }
    }
    if !reasoning.is_empty() {
        out.push_str(&format!("## Reasoning\n\n{reasoning}\n\n"));
    }
    if !text.is_empty() {
        out.push_str(&format!("## Response\n\n{text}\n\n"));
    }
    out
}

/// Import a JSONL dump produced by [`export_run_events_jsonl`] back into
/// `run_events` for `run_id`. Returns the number of rows inserted.
///
/// Lines whose `(run_id, sequence)` already exists are skipped
/// (`INSERT OR IGNORE`), so importing the same dump twice is safe. A line
/// carrying a different `run_id` is rejected instead of being silently
/// re-homed: importing another run's log would corrupt this run's replay.
#[tauri::command]
pub async fn import_run_events(
    state: State<'_, AppState>,
    run_id: String,
    jsonl: String,
) -> Result<usize, String> {
    Uuid::parse_str(&run_id).map_err(|_| format!("invalid run id: {run_id}"))?;
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM runs WHERE id = ?")
        .bind(&run_id)
        .fetch_optional(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    if exists.is_none() {
        return Err(format!("run not found: {run_id}"));
    }
    import_run_events_rows(state.storage.pool(), &run_id, &jsonl).await
}

/// Read a JSONL dump from `path` and import it, so the UI can offer
/// "import" with just a file-picker path and no separate file-read command.
#[tauri::command]
pub async fn import_run_events_file(
    state: State<'_, AppState>,
    run_id: String,
    path: String,
) -> Result<usize, String> {
    Uuid::parse_str(&run_id).map_err(|_| format!("invalid run id: {run_id}"))?;
    // A dump is bounded by `MAX_REPLAY_EVENTS` rows; 8 MiB leaves ample headroom
    // while refusing to read an arbitrary huge file into memory.
    const MAX_DUMP_BYTES: u64 = 8 * 1024 * 1024;
    let meta = std::fs::metadata(&path).map_err(crate::error::cmd_err)?;
    if meta.len() > MAX_DUMP_BYTES {
        return Err(format!(
            "dump is too large ({} bytes, max {MAX_DUMP_BYTES})",
            meta.len()
        ));
    }
    let jsonl = std::fs::read_to_string(&path).map_err(crate::error::cmd_err)?;
    import_run_events(state, run_id, jsonl).await
}

pub async fn import_run_events_rows(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    run_id: &str,
    jsonl: &str,
) -> Result<usize, String> {
    let limit = mixd_core::runs::events::MAX_REPLAY_EVENTS as usize;
    let lines: Vec<&str> = jsonl
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() > limit {
        return Err(format!(
            "too many events ({} lines, max {limit})",
            lines.len()
        ));
    }
    let mut inserted = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|err| format!("line {}: {err}", index + 1))?;
        if !value.is_object() {
            return Err(format!("line {}: expected a JSON object", index + 1));
        }
        if let Some(line_run) = value.get("run_id").and_then(|v| v.as_str()) {
            if line_run != run_id {
                return Err(format!(
                    "line {}: run_id {line_run} does not match {run_id}",
                    index + 1
                ));
            }
        }
        let kind = value
            .get("kind")
            .and_then(|v| v.as_str())
            .filter(|kind| !kind.is_empty())
            .ok_or_else(|| format!("line {}: missing kind", index + 1))?;
        let sequence = value
            .get("sequence")
            .and_then(|v| v.as_i64())
            .filter(|sequence| *sequence > 0)
            .ok_or_else(|| format!("line {}: missing positive sequence", index + 1))?;
        let data = value
            .get("data")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let data_text = serde_json::to_string(&data).map_err(crate::error::cmd_err)?;
        let created_at = value
            .get("created_at")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let result = sqlx::query(
            "INSERT OR IGNORE INTO run_events (id, run_id, sequence, kind, data, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(run_id)
        .bind(sequence)
        .bind(kind)
        .bind(&data_text)
        .bind(&created_at)
        .execute(pool)
        .await
        .map_err(crate::error::cmd_err)?;
        inserted += result.rows_affected() as usize;
    }
    Ok(inserted)
}

/// One run-event FTS hit for the Trajectory search surface.
#[derive(Debug, Clone, Serialize)]
pub struct RunEventSearchHit {
    pub run_id: String,
    pub conversation_id: Option<String>,
    pub sequence: i64,
    pub kind: String,
    /// FTS5 snippet around the match, for the result row.
    pub excerpt: String,
    pub created_at: String,
}

/// Search every run's event log through the FTS5 index (`0007`).
///
/// This is what makes the Trajectory view searchable across runs: tool
/// arguments, streamed text, errors and status transitions all match, where
/// transcript search (`search_messages`) only ever saw chat messages.
/// Queries shorter than three characters (and queries with no FTS5-safe run)
/// return an empty list.
#[tauri::command]
pub async fn search_run_events(
    state: State<'_, AppState>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<RunEventSearchHit>, String> {
    search_run_events_with(state.storage.pool(), &query, limit).await
}

pub async fn search_run_events_with(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    query: &str,
    limit: Option<u32>,
) -> Result<Vec<RunEventSearchHit>, String> {
    let Some(match_query) = mixd_core::retrieval::fts_match_query(query) else {
        return Ok(Vec::new());
    };
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let rows: Vec<(String, Option<String>, i64, String, String, String)> = sqlx::query_as(
        "SELECT s.run_id, r.conversation_id, s.sequence, s.kind, \
                snippet(run_events_search, 0, '', '', '…', 12), e.created_at \
         FROM run_events_search s \
         JOIN run_events e ON e.run_id = s.run_id AND e.sequence = s.sequence \
         LEFT JOIN runs r ON r.id = s.run_id \
         WHERE run_events_search MATCH ? \
         ORDER BY bm25(run_events_search) LIMIT ?",
    )
    .bind(match_query)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(
            |(run_id, conversation_id, sequence, kind, excerpt, created_at)| RunEventSearchHit {
                run_id,
                conversation_id,
                sequence,
                kind,
                excerpt,
                created_at,
            },
        )
        .collect())
}
