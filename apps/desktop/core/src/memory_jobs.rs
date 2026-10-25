//! Post-response memory auto formation (memory spec §6 自動形成).
//!
//! One job row per finished Run, enqueued right after the assistant's final
//! reply and drained by a 30-second tick ([`crate::schedule::MEMORY_JOB_TICK_SECS`])
//! so the response path never waits on the extractor. The shape mirrors the
//! Docker edition's `memory/jobs.py`: a durable `pending → running →
//! completed | retrying | failed | skipped` state machine with exponential
//! backoff, so a provider outage costs a retry, not a lost memory.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde_json::{json, Value};
use sqlx::Row;

use crate::memory::{self, ExtractedMemory, ExtractionApplied, MemoryService};
use crate::retrieval::EmbeddingSource;
use crate::runs::engine::{wire_model_for, ProviderRegistry};
use mixd_providers::{Message, Provider, ProviderRequest, StreamEvent};
use mixd_storage::Storage;

/// Spec §6: one extraction call may take up to 90 s before it is abandoned
/// to the retry path.
pub const EVAL_TIMEOUT: Duration = Duration::from_secs(90);

/// Spec §6: 「失敗時は 3 回まで」— three attempts total, then `failed`.
pub const MAX_ATTEMPTS: i64 = 3;

/// First retry delay. Backoff is exponential (`30 s, 60 s, …`, spec §6
/// 「指数的バックオフ」) rather than the Docker edition's linear `30 * attempts`.
pub const RETRY_BASE_SECS: i64 = 30;

/// Spec §6 input: the top 10 existing Memories are fed back so the extractor
/// can `REINFORCE` instead of duplicating them.
pub const EXISTING_LIMIT: usize = 10;

/// How many jobs one tick may run at once — the Docker scheduler's ceiling,
/// kept so a burst of finished Runs cannot fan out into unbounded provider
/// calls.
pub const MAX_CONCURRENT_JOBS: usize = 4;

/// How many jobs one tick looks at before leaving the rest for the next one.
pub const JOB_BATCH: usize = 8;

/// Spec §6 keeps the extractor's input small: the user turn and the reply.
const MAX_USER_CHARS: usize = 20_000;
const MAX_ASSISTANT_CHARS: usize = 10_000;

/// §6's call parameters: deterministic and short (`temperature 0`,
/// `max_tokens 800`).
const EXTRACTION_TEMPERATURE: f32 = 0.0;
const EXTRACTION_MAX_TOKENS: u32 = 800;

/// One row of `memory_processing_jobs`, with its `data` blob decoded.
#[derive(Debug, Clone)]
pub struct ExtractionJob {
    pub id: String,
    pub run_id: String,
    pub status: String,
    pub attempts: i64,
    pub data: Value,
}

impl ExtractionJob {
    /// Is the backoff window (or the first attempt) open right now?
    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        let Some(next) = self
            .data
            .get("nextAttemptAt")
            .and_then(|v| v.as_str())
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        else {
            // No recorded window: due as soon as it is claimable.
            return true;
        };
        next.with_timezone(&Utc) <= now
    }

    fn text(&self, key: &str) -> anyhow::Result<&str> {
        self.data
            .get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("memory job is missing {key}"))
    }
}

/// What one [`JobRunner::process_job`] did, for callers and tests alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// The reply was applied: these rows are new, these were reinforced.
    Applied {
        created: Vec<String>,
        reinforced: Vec<String>,
    },
    /// The Run is gone or `memory_auto_formation` is off.
    Skipped,
    /// Not claimable yet: the backoff window is still closed, or another
    /// worker owns it.
    NotDue,
    /// No such job.
    Gone,
    /// The attempt failed and will be retried after an exponential backoff.
    RetryScheduled { attempts: i64, error: String },
    /// Three attempts have failed; the job is terminal.
    Failed { attempts: i64, error: String },
}

/// Queue one finished Run's turn for extraction (spec §6 契機).
///
/// Idempotent per Run — `memory_processing_jobs` has `UNIQUE(run_id)` — and
/// deliberately tolerant of a Run with no `runs` row (a sub-agent turn): the
/// foreign key means such a run simply has no extraction, which is what
/// "one user turn, one extraction" implies. Returns whether a row was
/// inserted.
pub async fn enqueue(
    storage: &Storage,
    run_id: &str,
    user: &str,
    assistant: &str,
) -> anyhow::Result<bool> {
    let run: Option<(String,)> = sqlx::query_as("SELECT id FROM runs WHERE id = ?")
        .bind(run_id)
        .fetch_optional(storage.pool())
        .await?;
    if run.is_none() {
        return Ok(false);
    }
    let queued: Option<(String,)> =
        sqlx::query_as("SELECT id FROM memory_processing_jobs WHERE run_id = ?")
            .bind(run_id)
            .fetch_optional(storage.pool())
            .await?;
    if queued.is_some() {
        return Ok(false);
    }

    let now = Utc::now();
    // Redaction runs before the text is stored (spec §8): the extractor sees
    // the exchange with credential values already replaced.
    let data = json!({
        "userContent": memory::redact_sensitive(&clip_chars(user, MAX_USER_CHARS)),
        "assistantContent": memory::redact_sensitive(&clip_chars(assistant, MAX_ASSISTANT_CHARS)),
        "nextAttemptAt": now.to_rfc3339(),
    });
    let result = sqlx::query(
        "INSERT INTO memory_processing_jobs \
         (id, owner_id, data, run_id, status, attempts, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'pending', 0, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(data.to_string())
    .bind(run_id)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .execute(storage.pool())
    .await;
    match result {
        Ok(_) => Ok(true),
        // Lost the race against a concurrent enqueue: still exactly one job.
        Err(err) if is_unique_violation(&err) => Ok(false),
        Err(err) => Err(err.into()),
    }
}

/// The jobs a tick should consider: claimable statuses, oldest first, with
/// each job's backoff window applied afterwards (the window lives in JSON,
/// so it cannot be a SQL predicate without hiding the rows behind it).
pub async fn due_jobs(storage: &Storage, limit: usize) -> Vec<ExtractionJob> {
    let rows = match sqlx::query(
        "SELECT id, run_id, status, attempts, data FROM memory_processing_jobs \
         WHERE status IN ('pending', 'retrying') ORDER BY created_at LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(storage.pool())
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(%err, "reading memory processing jobs failed");
            return Vec::new();
        }
    };
    let now = Utc::now();
    rows.into_iter()
        .filter_map(row_to_job)
        .filter(|job| job.is_due(now))
        .collect()
}

fn row_to_job(row: sqlx::sqlite::SqliteRow) -> Option<ExtractionJob> {
    Some(ExtractionJob {
        id: row.try_get("id").ok()?,
        run_id: row.try_get("run_id").ok()?,
        status: row.try_get("status").ok()?,
        attempts: row.try_get("attempts").ok()?,
        data: serde_json::from_str(&row.try_get::<String, _>("data").ok()?).unwrap_or(Value::Null),
    })
}

/// The Run and its job together: everything one attempt needs.
struct ExtractionPlan {
    provider_id: String,
    /// The model name the adapter expects, with any `<provider>:` prefix
    /// stripped (the stored id is `<provider_id>:<model>`).
    model: String,
    user: String,
    assistant: String,
}

/// Drains the queue for one [`Storage`] / [`ProviderRegistry`] pair.
///
/// The app shell builds one per tick: the registry is the engine's (the same
/// providers a Run just used), the embedding source is resolved fresh so a
/// provider or settings change takes effect without a restart.
#[derive(Clone)]
pub struct JobRunner {
    pub storage: Arc<Storage>,
    pub providers: Arc<ProviderRegistry>,
    /// Embeds the rows this run creates. `None` — or an `embed` that fails —
    /// leaves them to rank lexically (spec §5.2), so it is never a blocker.
    pub embedding: Option<EmbeddingSource>,
    /// Per-attempt ceiling; injectable so a hanging provider is testable in
    /// milliseconds instead of 90 seconds.
    pub timeout: Duration,
}

impl JobRunner {
    pub fn new(storage: Arc<Storage>, providers: Arc<ProviderRegistry>) -> Self {
        Self {
            storage,
            providers,
            embedding: None,
            timeout: EVAL_TIMEOUT,
        }
    }

    pub fn with_embedding(mut self, embedding: Option<EmbeddingSource>) -> Self {
        self.embedding = embedding;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Claim and run `id`. Safe to call for a job that is not claimable:
    /// nothing is changed and [`JobOutcome::NotDue`] comes back.
    pub async fn process_job(&self, id: &str) -> JobOutcome {
        let Some(job) = self.load_job(id).await else {
            return JobOutcome::Gone;
        };
        if !matches!(job.status.as_str(), "pending" | "retrying") {
            return JobOutcome::Gone;
        }
        if !job.is_due(Utc::now()) {
            return JobOutcome::NotDue;
        }

        // Skips happen before the claim, so switching the setting off never
        // spends an attempt (spec §6: `memory_auto_formation=false` で停止).
        let plan = match self.plan(&job).await {
            Ok(plan) => plan,
            Err(reason) => {
                tracing::debug!(job = %job.id, %reason, "memory extraction job skipped");
                self.mark(&job, "skipped", job.data.clone()).await;
                return JobOutcome::Skipped;
            }
        };
        if !memory::auto_formation_enabled(self.storage.pool()).await {
            self.mark(&job, "skipped", job.data.clone()).await;
            return JobOutcome::Skipped;
        }

        if !self.claim(&job).await {
            return JobOutcome::NotDue;
        }
        let attempts = job.attempts + 1;

        match self.extract(&job, &plan).await {
            Ok(applied) => {
                self.embed_created(&applied).await;
                // §4 source (3) runs *after* embedding: it is §5's embedding
                // neighbourhood, so the rows have to carry a vector by now, and
                // it is gated on `memory_link_embedding_neighbors` — **既定オフ**
                // (§4), because links made here grow with the square of the
                // store. Source (1) already ran inside `apply_extraction`,
                // source (2) belongs to search and is not reached here.
                if let Err(err) = crate::memory_links::embedding_neighbor_links_if_enabled(
                    &self.storage,
                    &applied.created,
                )
                .await
                {
                    tracing::warn!(%err, "embedding-neighbour link suggestion failed");
                }
                let mut data = job.data.clone();
                data["candidateCount"] = json!(applied.created.len() + applied.reinforced.len());
                data["completedAt"] = json!(Utc::now().to_rfc3339());
                if let Some(object) = data.as_object_mut() {
                    object.remove("error");
                }
                self.mark(&job, "completed", data).await;
                JobOutcome::Applied {
                    created: applied.created,
                    reinforced: applied.reinforced,
                }
            }
            Err(err) => {
                let error = err.to_string();
                let mut data = job.data.clone();
                data["error"] = json!(error);
                if attempts >= MAX_ATTEMPTS {
                    self.mark(&job, "failed", data).await;
                    JobOutcome::Failed { attempts, error }
                } else {
                    let backoff = RETRY_BASE_SECS * 2_i64.pow((attempts - 1) as u32);
                    data["nextAttemptAt"] =
                        json!((Utc::now() + chrono::Duration::seconds(backoff)).to_rfc3339());
                    self.mark(&job, "retrying", data).await;
                    JobOutcome::RetryScheduled { attempts, error }
                }
            }
        }
    }

    /// Load a job by id.
    async fn load_job(&self, id: &str) -> Option<ExtractionJob> {
        let row = sqlx::query(
            "SELECT id, run_id, status, attempts, data FROM memory_processing_jobs WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.storage.pool())
        .await
        .ok()
        .flatten()?;
        row_to_job(row)
    }

    /// Resolve everything the attempt needs, or explain why the job must be
    /// skipped: a Run that was deleted, or one whose row never existed.
    async fn plan(&self, job: &ExtractionJob) -> anyhow::Result<ExtractionPlan> {
        let row: Option<(String,)> = sqlx::query_as("SELECT data FROM runs WHERE id = ?")
            .bind(&job.run_id)
            .fetch_optional(self.storage.pool())
            .await?;
        let (raw,) = row.ok_or_else(|| anyhow::anyhow!("the run no longer exists"))?;
        let run: Value = serde_json::from_str(&raw)?;
        let model = run
            .get("modelId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("the run has no modelId"))?;
        // `providerId` was added to the run row for exactly this; older rows
        // (and anything else that wrote one) fall back to the id's prefix.
        let provider_id = run
            .get("providerId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                model
                    .split_once(':')
                    .map(|(p, _)| p)
                    .unwrap_or_default()
                    .to_string()
            });
        Ok(ExtractionPlan {
            model: wire_model_for(model, &provider_id).to_string(),
            provider_id,
            user: job.text("userContent")?.to_string(),
            assistant: job.text("assistantContent")?.to_string(),
        })
    }

    /// Move `pending`/`retrying` → `running` and spend one attempt.
    ///
    /// Returns `false` when the row was claimed by someone else (or vanished)
    /// between the read and the update.
    async fn claim(&self, job: &ExtractionJob) -> bool {
        let result = sqlx::query(
            "UPDATE memory_processing_jobs \
             SET status = 'running', attempts = attempts + 1, updated_at = ? \
             WHERE id = ? AND status IN ('pending', 'retrying')",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(&job.id)
        .execute(self.storage.pool())
        .await;
        matches!(result, Ok(row) if row.rows_affected() == 1)
    }

    /// Write the job's next state. Best effort: the outcome has already been
    /// decided, and a job row that cannot be updated must not turn a
    /// completed extraction into an error.
    async fn mark(&self, job: &ExtractionJob, status: &str, data: Value) {
        mixd_storage::log_write(
            sqlx::query(
                "UPDATE memory_processing_jobs SET status = ?, data = ?, updated_at = ? \
                 WHERE id = ?",
            )
            .bind(status)
            .bind(data.to_string())
            .bind(Utc::now().to_rfc3339())
            .bind(&job.id)
            .execute(self.storage.pool())
            .await,
            "update memory processing job",
        );
    }

    /// One full attempt: extract, then apply (spec §6 入力 → 出力 → 保存).
    async fn extract(
        &self,
        job: &ExtractionJob,
        plan: &ExtractionPlan,
    ) -> anyhow::Result<ExtractionApplied> {
        let items = self.evaluate(plan).await?;
        memory::apply_extraction(&self.storage, &items, &job.run_id).await
    }

    /// Call the extractor and parse its reply.
    ///
    /// The whole call — opening the stream, reading it, the parse — sits
    /// under one timeout: a provider that hangs mid-response must reach the
    /// retry path like one that never answers.
    async fn evaluate(&self, plan: &ExtractionPlan) -> anyhow::Result<Vec<ExtractedMemory>> {
        let provider = self
            .providers
            .get(&plan.provider_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("provider not configured: {}", plan.provider_id))?;

        let existing = MemoryService::list(&self.storage, EXISTING_LIMIT)
            .await?
            .into_iter()
            .map(|item| item.content)
            .collect::<Vec<_>>();
        let payload = json!({
            "user": plan.user,
            "assistant": plan.assistant,
            "existing": existing,
        });
        let request = ProviderRequest {
            model: plan.model.clone(),
            // Spec §6 input: system = the shared extraction prompt, user =
            // the turn as JSON.
            messages: vec![Message::text("user", payload.to_string())],
            system: Some(memory::EXTRACTION_PROMPT.to_string()),
            tools: Arc::new(Vec::new()),
            temperature: Some(EXTRACTION_TEMPERATURE),
            top_p: None,
            max_tokens: Some(EXTRACTION_MAX_TOKENS),
            reasoning: None,
            stream: true,
        };

        let text = tokio::time::timeout(self.timeout, complete(provider.as_ref(), request))
            .await
            .map_err(|_| {
                anyhow::anyhow!("memory extraction timed out after {:?}", self.timeout)
            })??;
        memory::parse_extraction(&text)
    }

    /// Embed every row this attempt created (spec §5). Reinforced rows keep
    /// the vector they already had: their text did not change.
    async fn embed_created(&self, applied: &ExtractionApplied) {
        let Some(source) = self.embedding.as_ref() else {
            return;
        };
        for id in &applied.created {
            match MemoryService::embed_item(&self.storage, source, id).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::debug!(memory = %id, "embedding unavailable; the memory ranks lexically")
                }
                Err(err) => {
                    tracing::warn!(memory = %id, %err, "storing the memory embedding failed")
                }
            }
        }
    }
}

/// Read one extraction reply to `Finished`, accumulating its text.
async fn complete(provider: &dyn Provider, request: ProviderRequest) -> anyhow::Result<String> {
    let mut stream = provider.stream(request).await?;
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::TextDelta(delta) => text.push_str(&delta),
            StreamEvent::Finished { .. } => return Ok(text),
            StreamEvent::Error(err) => anyhow::bail!(err),
            _ => {}
        }
    }
    // A stream that stops mid-answer is a failure, not a short reply — the
    // same rule the run engine applies to a dropped connection.
    anyhow::bail!("the provider stream ended without a completion signal")
}

fn clip_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .map(|db| db.message().contains("UNIQUE constraint failed"))
        .unwrap_or(false)
}
