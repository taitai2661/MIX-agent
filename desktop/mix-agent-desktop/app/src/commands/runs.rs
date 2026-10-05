use crate::AppState;
use mixd_core::runs::state::RunView;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub async fn get_run(state: State<'_, AppState>, id: String) -> Result<RunView, String> {
    let run_id = Uuid::parse_str(&id).map_err(crate::error::cmd_err)?;
    state
        .engine
        .run_state(run_id)
        .await
        .ok_or_else(|| "run not found".to_string())
}

#[tauri::command]
pub async fn list_runs(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<RunView>, String> {
    let conversation_id = Uuid::parse_str(&conversation_id).map_err(crate::error::cmd_err)?;
    Ok(state.engine.list_runs(conversation_id).await)
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
    let rows: Vec<RunEventRow> = sqlx::query_as(
        "SELECT run_id, sequence, kind, data, created_at \
         FROM run_events WHERE run_id = ? ORDER BY sequence ASC",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .map_err(crate::error::cmd_err)?;
    Ok(rows
        .into_iter()
        .map(|(run_id, sequence, kind, data, created_at)| RunEventView {
            run_id,
            sequence,
            kind,
            data: serde_json::from_str(&data).unwrap_or(serde_json::Value::Null),
            created_at,
        })
        .collect())
}

type RunEventRow = (String, i64, String, String, String);

#[tauri::command]
pub async fn cancel_run(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let run_id = Uuid::parse_str(&id).map_err(crate::error::cmd_err)?;
    state.engine.cancel(run_id).await;
    sqlx::query(
        "UPDATE runs SET status = 'cancelled' \
         WHERE id = ? AND status NOT IN ('completed', 'failed', 'cancelled', 'interrupted')",
    )
    .bind(run_id.to_string())
    .execute(state.storage.pool())
    .await
    .map_err(crate::error::cmd_err)?;
    state
        .engine
        .emit(
            run_id,
            "status",
            serde_json::json!({ "status": "cancelled" }),
        )
        .await;
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
    // `executing` before the interruption.
    for action in &input.unknown_actions {
        let status = match action.action.as_str() {
            "retry" => "pending",
            "discard" => "discarded",
            _ => "unknown",
        };
        sqlx::query("UPDATE tool_calls SET status = ? WHERE id = ? AND run_id = ?")
            .bind(status)
            .bind(&action.tool_call_id)
            .bind(run_id.to_string())
            .execute(state.storage.pool())
            .await
            .map_err(crate::error::cmd_err)?;
    }

    // Load the Run's original request parameters so the loop can restart.
    let row: Option<(String, String, String, Option<String>)> =
        sqlx::query_as::<_, (String, String, String, Option<String>)>(
            "SELECT conversation_id, json_extract(data, '$.modelId'), \
             json_extract(data, '$.mode'), json_extract(data, '$.workspaceId') \
             FROM runs WHERE id = ?",
        )
        .bind(run_id.to_string())
        .fetch_optional(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    let (conversation_id, model_id, mode_str, workspace_id) = row.ok_or("run not found")?;
    let mode = mixd_core::Mode::parse(&mode_str).ok_or("invalid mode")?;
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
    .map_err(|err| err.to_string())?;
    let tools: Vec<mixd_providers::ToolSpec> = state.engine.tool_specs().await;

    // Re-apply the project context so a resumed run keeps its folder scope and
    // instructions even if the original request predates the project feature.
    let project =
        crate::commands::projects::load_project_runtime(state.storage.pool(), &conversation_id)
            .await?;

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
        temperature: None,
        reasoning: None,
        max_tokens: None,
        tools,
        history: Vec::new(),
        user_message: String::new(),
        resume_from: Some(snapshot),
    };

    sqlx::query("UPDATE runs SET status = 'running' WHERE id = ?")
        .bind(run_id.to_string())
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
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
        let result = engine.execute(request).await;
        // Both arms matter: swallowing `Err` here left the row `running`
        // forever, and it is also counted as an active run by diagnostics.
        let status = match &result {
            Ok(outcome) => outcome.status.as_str().to_string(),
            Err(err) => {
                tracing::error!(?err, %run_id, "resumed run failed");
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
        if let Err(err) = result {
            let message = err.to_string();
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
