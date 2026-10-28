//! Schedule daemon.
//!
//! Wakes on a fixed tick, finds every enabled job whose cron expression is due,
//! and starts a background Run for it. The due check is derived from
//! `lastRunAt` rather than from "have I already fired this minute", so a job
//! that was missed while the app was closed runs once on the next launch instead
//! of being skipped or replayed in a burst.

use chrono::{DateTime, Utc};
use mixd_core::memory_jobs::{JobOutcome, JobRunner, JOB_BATCH, MAX_CONCURRENT_JOBS};
use mixd_core::schedule::{CronSchedule, MEMORY_DECAY_TICK_SECS, MEMORY_JOB_TICK_SECS, TICK_SECS};
use mixd_storage::Storage;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::AppState;

/// One row of `scheduled_jobs`, flattened out of the JSON blob.
#[derive(Debug, Clone)]
struct Job {
    id: String,
    name: String,
    cron: String,
    prompt: String,
    conversation_id: Option<String>,
    model_id: Option<String>,
    mode: Option<String>,
    enabled: bool,
    created_at: DateTime<Utc>,
    last_run_at: Option<DateTime<Utc>>,
}

/// Read every job, skipping rows whose stored cron expression no longer parses
/// so one bad entry cannot stall the whole daemon.
async fn load_jobs(storage: &Storage) -> Vec<Job> {
    use sqlx::Row;
    let Ok(rows) = sqlx::query("SELECT id, data, created_at FROM scheduled_jobs")
        .fetch_all(storage.pool())
        .await
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in rows {
        let id: String = row.try_get("id").unwrap_or_default();
        let raw: String = row.try_get::<String, _>("data").unwrap_or_default();
        let data: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
        let cron = data
            .get("cron")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if CronSchedule::parse(&cron).is_err() {
            tracing::warn!(%id, %cron, "skipping job with an unparseable cron expression");
            continue;
        }
        out.push(Job {
            name: data
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("(unnamed)")
                .to_string(),
            prompt: data
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            conversation_id: data
                .get("conversationId")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            model_id: data
                .get("modelId")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            mode: data
                .get("mode")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            enabled: data
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            last_run_at: data
                .get("lastRunAt")
                .and_then(|v| v.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc)),
            created_at: row
                .try_get::<String, _>("created_at")
                .ok()
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(Utc::now),
            cron,
            id,
        });
    }
    out
}

/// Is this job due at `now`?
///
/// A job is due when a fire time falls in the half-open window
/// `(anchor, now]`, where the anchor is the last run or, for a job that has
/// never run, its creation time. Marking `lastRunAt = now` when a job fires moves
/// the anchor forward, so the same slot is never run twice, and a job that was
/// missed while the app was closed catches up with exactly one run on the next
/// launch instead of replaying every missed slot.
fn is_due(job: &Job, now: DateTime<Utc>) -> bool {
    if !job.enabled || job.prompt.trim().is_empty() {
        return false;
    }
    let Ok(sched) = CronSchedule::parse(&job.cron) else {
        return false;
    };
    let anchor = job.last_run_at.unwrap_or(job.created_at);
    if now <= anchor {
        return false;
    }
    match sched.next_after(anchor) {
        Some(fire) => fire <= now,
        // A schedule with no future occurrence (e.g. February 30th) never fires.
        None => false,
    }
}

/// Record the fire time so the same slot is not run twice.
async fn mark_ran(storage: &Storage, job: &Job, now: DateTime<Utc>) {
    mixd_storage::log_write(
        sqlx::query(
            "UPDATE scheduled_jobs \
             SET data = json_set(data, '$.lastRunAt', ?, '$.lastStatus', 'started') \
             WHERE id = ?",
        )
        .bind(now.to_rfc3339())
        .bind(&job.id)
        .execute(storage.pool())
        .await,
        "mark scheduled job as run",
    );
}

/// Create the conversation a job should post into, reusing the configured one.
async fn target_conversation(state: &AppState, job: &Job) -> Result<String, String> {
    if let Some(id) = job.conversation_id.as_ref().filter(|s| !s.is_empty()) {
        return Ok(id.clone());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let model = job.model_id.clone().unwrap_or_default();
    let data = serde_json::json!({
        "title": job.name,
        "mode": job.mode.clone().unwrap_or_else(|| "chat".into()),
        "modelId": model,
        "createdAt": Utc::now().to_rfc3339(),
        "updatedAt": Utc::now().to_rfc3339(),
        "scheduled": true,
    });
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .execute(state.storage.pool())
        .await
        .map_err(crate::error::cmd_err)?;
    // Remember it so later runs reuse the same thread.
    mixd_storage::log_write(
        sqlx::query(
            "UPDATE scheduled_jobs SET data = json_set(data, '$.conversationId', ?) WHERE id = ?",
        )
        .bind(&id)
        .bind(&job.id)
        .execute(state.storage.pool())
        .await,
        "record scheduled job conversation",
    );
    Ok(id)
}

/// Run every job that is due right now. Exposed for tests.
pub async fn tick(state: &AppState) -> usize {
    let now = Utc::now();
    let jobs = load_jobs(state.storage.as_ref()).await;
    let mut started = 0usize;
    for job in jobs {
        if !is_due(&job, now) {
            continue;
        }
        // The fire marker is written only after the run is accepted: marking
        // before dispatch turned every failed start into a silently skipped
        // slot. Failures still advance `lastRunAt` (with an error status) so a
        // permanently broken job does not retry on every tick.
        match target_conversation(state, &job).await {
            Ok(conversation_id) => {
                let input = crate::commands::conversations::SendMessageInput {
                    conversation_id,
                    content: job.prompt.clone(),
                    model_id: job.model_id.clone(),
                    mode: job.mode.clone(),
                    workspace_id: None,
                    attachments: Vec::new(),
                    reasoning: None,
                    temperature: None,
                    max_tokens: None,
                    temporary_mode: false,
                    allow_tools: false,
                };
                match crate::commands::conversations::dispatch_run(state, input).await {
                    Ok(out) => {
                        mark_ran(state.storage.as_ref(), &job, now).await;
                        started += 1;
                        tracing::info!(job = %job.name, run = %out.run_id, "scheduled run started");
                    }
                    Err(err) => {
                        tracing::error!(job = %job.name, %err, "scheduled run could not start");
                        mixd_storage::log_write(
                            sqlx::query(
                                "UPDATE scheduled_jobs \
                                 SET data = json_set(data, '$.lastRunAt', ?, '$.lastStatus', 'error', '$.lastError', ?) \
                                 WHERE id = ?",
                            )
                            .bind(now.to_rfc3339())
                            .bind(&err)
                            .bind(&job.id)
                            .execute(state.storage.pool())
                            .await,
                            "record scheduled job error",
                        );
                    }
                }
            }
            Err(err) => {
                tracing::error!(job = %job.name, %err, "no conversation for scheduled run");
                mixd_storage::log_write(
                    sqlx::query(
                        "UPDATE scheduled_jobs \
                         SET data = json_set(data, '$.lastRunAt', ?, '$.lastStatus', 'error', '$.lastError', ?) \
                         WHERE id = ?",
                    )
                    .bind(now.to_rfc3339())
                    .bind(&err)
                    .bind(&job.id)
                    .execute(state.storage.pool())
                    .await,
                    "record scheduled job conversation error",
                );
            }
        }
    }
    started
}

/// Fire one job immediately ("Run now"), reusing the same dispatch path as the
/// cron tick so a manual run creates/uses the same conversation and advances
/// `lastRunAt` (otherwise the daemon would fire the same slot again seconds
/// later).
pub async fn run_now(state: &AppState, id: &str) -> Result<String, String> {
    let jobs = load_jobs(state.storage.as_ref()).await;
    let job = jobs
        .into_iter()
        .find(|job| job.id == id)
        .ok_or_else(|| format!("job not found: {id}"))?;
    if job.prompt.trim().is_empty() {
        return Err("this job has no prompt".into());
    }
    let conversation_id = target_conversation(state, &job).await?;
    let input = crate::commands::conversations::SendMessageInput {
        conversation_id,
        content: job.prompt.clone(),
        model_id: job.model_id.clone(),
        mode: job.mode.clone(),
        workspace_id: None,
        attachments: Vec::new(),
        reasoning: None,
        temperature: None,
        max_tokens: None,
        temporary_mode: false,
        allow_tools: false,
    };
    let out = crate::commands::conversations::dispatch_run(state, input).await?;
    mark_ran(state.storage.as_ref(), &job, Utc::now()).await;
    Ok(out.run_id)
}

/// Start the daemon on Tauri's global async runtime.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            {
                let app = handle.clone();
                let state = app.state::<AppState>();
                let count = tick(&state).await;
                if count > 0 {
                    let _ = handle.emit("scheduled_runs_started", count);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(TICK_SECS)).await;
        }
    });
    install_memory_jobs(app);
    install_memory_decay(app);
}

/// Drain the memory auto-formation queue (memory spec §6 自動形成).
///
/// A loop of its own rather than a step in the cron tick above: an
/// extraction attempt may hold the runner for up to 90 seconds, and a
/// cron job becoming due must not wait behind it. At most
/// [`MAX_CONCURRENT_JOBS`] attempts run at once — the ceiling keeps a burst
/// of finished conversations from fanning out into unbounded provider calls —
/// and an id that is already running is simply left for the tick that
/// finishes it.
fn install_memory_jobs<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let active = Arc::new(tokio::sync::Mutex::new(
            std::collections::HashSet::<String>::new(),
        ));
        loop {
            {
                let state = handle.state::<AppState>();
                // Resolved per tick: the resolver caches its answer and
                // settings/provider changes invalidate it, so a new provider
                // embeds newly extracted memories without a restart.
                let embedding = state.embeddings.resolve().await;
                let runner = JobRunner::new(state.storage.clone(), state.engine.providers.clone())
                    .with_embedding(embedding);
                let due = mixd_core::memory_jobs::due_jobs(state.storage.as_ref(), JOB_BATCH).await;
                for job in due {
                    let mut in_flight = active.lock().await;
                    if in_flight.len() >= MAX_CONCURRENT_JOBS {
                        break;
                    }
                    if !in_flight.insert(job.id.clone()) {
                        continue;
                    }
                    drop(in_flight);

                    let runner = runner.clone();
                    let active = active.clone();
                    let id = job.id;
                    tokio::spawn(async move {
                        match runner.process_job(&id).await {
                            JobOutcome::Applied {
                                created,
                                reinforced,
                            } => tracing::info!(
                                job = %id,
                                created = created.len(),
                                reinforced = reinforced.len(),
                                "memory extraction applied"
                            ),
                            JobOutcome::RetryScheduled { attempts, error } => tracing::debug!(
                                job = %id,
                                attempts,
                                %error,
                                "memory extraction attempt failed; retrying"
                            ),
                            JobOutcome::Failed { attempts, error } => tracing::warn!(
                                job = %id,
                                attempts,
                                %error,
                                "memory extraction failed permanently"
                            ),
                            JobOutcome::Skipped => {
                                tracing::debug!(job = %id, "memory extraction skipped")
                            }
                            JobOutcome::NotDue | JobOutcome::Gone => {}
                        }
                        active.lock().await.remove(&id);
                    });
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(MEMORY_JOB_TICK_SECS)).await;
        }
    });
}

/// Weaken idle memories once a day (memory spec §3 減衰).
///
/// A loop of its own rather than a step in the cron tick or the extraction
/// tick: decay is a daily job (§3's half-life is 180 days and it refuses rows
/// younger than half a day), and running it on the 20 s / 30 s cadences would
/// re-read rows it must not touch. The first pass fires at startup — harmless,
/// because a row's own `last_decay_at` / `updated_at` anchor is what decides
/// whether it may be decayed at all, not how often the loop wakes.
///
/// `pinned` rows, `updated_at` and the revision history are handled inside
/// [`mixd_core::memory::MemoryService::decay`]; this loop only decides *when*.
fn install_memory_decay<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            {
                let state = handle.state::<AppState>();
                match mixd_core::memory::MemoryService::decay(state.storage.as_ref()).await {
                    Ok(touched) if touched > 0 => {
                        tracing::info!(rows = touched, "memory decay applied");
                    }
                    Ok(_) => {}
                    // A failed pass must not stop the loop: the next day's
                    // window re-applies from each row's own anchor.
                    Err(err) => tracing::warn!(%err, "memory decay failed"),
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(MEMORY_DECAY_TICK_SECS)).await;
        }
    });
}

/// Test-only helper: expose the due check.
pub fn due_for_test(
    job_cron: &str,
    created_at: DateTime<Utc>,
    last_run: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    is_due(
        &Job {
            id: "test".into(),
            name: "test".into(),
            cron: job_cron.into(),
            prompt: "hello".into(),
            conversation_id: None,
            model_id: None,
            mode: None,
            enabled: true,
            created_at,
            last_run_at: last_run,
        },
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, h, m, s).unwrap()
    }

    #[test]
    fn a_new_job_becomes_due_after_its_first_slot() {
        // Hourly job created at 10:10; first slot is 11:00.
        assert!(!due_for_test("0 * * * *", t(10, 10, 0), None, t(10, 59, 0)));
        assert!(due_for_test("0 * * * *", t(10, 10, 0), None, t(11, 0, 0)));
    }

    #[test]
    fn firing_marks_the_slot_consumed_so_it_does_not_repeat() {
        // lastRunAt == 11:00 means the 11:00 slot is already used.
        assert!(!due_for_test(
            "0 * * * *",
            t(10, 0, 0),
            Some(t(11, 0, 0)),
            t(11, 30, 0)
        ));
        assert!(due_for_test(
            "0 * * * *",
            t(10, 0, 0),
            Some(t(11, 0, 0)),
            t(12, 0, 0)
        ));
    }

    #[test]
    fn missed_runs_catch_up_exactly_once() {
        // App was closed for three days; the hourly job should run once, not
        // replay 72 slots.
        let last = t(0, 0, 0);
        assert!(due_for_test(
            "0 * * * *",
            last,
            Some(last),
            t(0, 30, 0) + chrono::Duration::days(3)
        ));
    }

    #[test]
    fn disabled_and_promptless_jobs_never_run() {
        let now = t(11, 0, 0);
        let job_due = || t(10, 0, 0);
        assert!(due_for_test("0 * * * *", job_due(), None, now));

        // Disabled via the same predicate with enabled=false.
        let disabled = Job {
            id: "x".into(),
            name: "x".into(),
            cron: "0 * * * *".into(),
            prompt: "hi".into(),
            conversation_id: None,
            model_id: None,
            mode: None,
            enabled: false,
            created_at: t(10, 0, 0),
            last_run_at: None,
        };
        assert!(!is_due(&disabled, now), "disabled jobs must not run");

        let empty_prompt = Job {
            enabled: true,
            prompt: "   ".into(),
            ..disabled
        };
        assert!(!is_due(&empty_prompt, now), "empty prompts must not run");
    }

    #[test]
    fn an_invalid_cron_never_fires() {
        assert!(!due_for_test("not a cron", t(0, 0, 0), None, t(12, 0, 0)));
    }

    #[test]
    fn clock_going_backwards_does_not_fire() {
        assert!(!due_for_test("0 * * * *", t(12, 0, 0), None, t(11, 0, 0)));
    }
}
