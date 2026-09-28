use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
pub struct ScheduleJobView {
    pub id: String,
    pub name: String,
    pub cron: String,
    pub enabled: bool,
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Next fire time, when the expression still resolves.
    pub next_run_at: Option<String>,
}

#[tauri::command]
pub async fn list_jobs(state: State<'_, AppState>) -> Result<Vec<ScheduleJobView>, String> {
    let pool = state.storage.pool();
    let rows: Vec<(String, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, json_extract(data, '$.name') AS name, json_extract(data, '$.cron') AS cron, \
         json_extract(data, '$.enabled') AS enabled, json_extract(data, '$.lastRunAt') AS last_run_at \
         FROM scheduled_jobs ORDER BY json_extract(data, '$.name')",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(id, name, cron, enabled, last_run_at)| {
            let last = last_run_at
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|t| t.with_timezone(&chrono::Utc));
            let enabled = enabled != 0;
            let next_run_at = if enabled {
                mixd_core::schedule::CronSchedule::parse(&cron)
                    .ok()
                    .and_then(|s| s.next_after(last.unwrap_or_else(chrono::Utc::now)))
                    .map(|t| t.to_rfc3339())
            } else {
                None
            };
            ScheduleJobView {
                id,
                name,
                cron,
                enabled,
                last_run_at: last,
                next_run_at,
            }
        })
        .collect())
}

#[derive(Deserialize)]
pub struct ScheduleJobCreate {
    pub name: String,
    pub cron: String,
    pub prompt: String,
    pub conversation_id: Option<String>,
    pub model_id: Option<String>,
    pub mode: Option<String>,
}

/// Validate a cron expression, so a typo is reported at creation time rather
/// than silently producing a job that never fires.
fn validate_cron(expr: &str) -> Result<mixd_core::schedule::CronSchedule, String> {
    mixd_core::schedule::CronSchedule::parse(expr).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_job(
    state: State<'_, AppState>,
    input: ScheduleJobCreate,
) -> Result<ScheduleJobView, String> {
    let schedule = validate_cron(&input.cron)?;
    if input.prompt.trim().is_empty() {
        return Err("a schedule needs a prompt".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let data = serde_json::json!({
        "name": input.name,
        "cron": input.cron,
        "prompt": input.prompt,
        "conversationId": input.conversation_id,
        "modelId": input.model_id,
        "mode": input.mode.unwrap_or_else(|| "chat".into()),
        "enabled": true,
    });
    sqlx::query("INSERT INTO scheduled_jobs (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(state.storage.pool())
        .await
        .map_err(|e| e.to_string())?;
    // Surface the first fire time so the user can sanity-check the expression.
    let next = schedule
        .next_after(chrono::Utc::now())
        .map(|t| t.to_rfc3339());
    Ok(ScheduleJobView {
        id,
        name: input.name,
        cron: input.cron,
        enabled: true,
        last_run_at: None,
        next_run_at: next,
    })
}

/// Enable or disable a job without deleting it.
#[tauri::command]
pub async fn set_job_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE scheduled_jobs SET data = json_set(data, '$.enabled', json(?1)) WHERE id = ?",
    )
    .bind(if enabled { "true" } else { "false" })
    .bind(&id)
    .execute(state.storage.pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Preview the next fire times for an expression without saving anything.
#[tauri::command]
pub async fn preview_cron(cron: String) -> Result<Vec<String>, String> {
    let schedule = validate_cron(&cron)?;
    let mut cursor = chrono::Utc::now();
    let mut out = Vec::with_capacity(5);
    for _ in 0..5 {
        match schedule.next_after(cursor) {
            Some(next) => {
                out.push(next.to_rfc3339());
                cursor = next;
            }
            None => break,
        }
    }
    if out.is_empty() {
        return Err("this expression never fires".into());
    }
    Ok(out)
}

#[tauri::command]
pub async fn delete_job(state: State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM scheduled_jobs WHERE id = ?")
        .bind(&id)
        .execute(state.storage.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
