//! Run Engine: orchestrates the per-Run state machine, drives Provider streams,
//! dispatches Tool calls, and emits the SSE-equivalent event envelope consumed
//! by the Tauri command surface.
//!
//! ```text
//! queued → running → waiting_approval → running → completed
//!                  ↘ budget_extension_pending → running / completed
//!                  ↘ interrupted / failed / cancelled
//! ```
//!
//! The loop implements the Docker edition's Agent autonomy contract:
//! per-step Tool dispatch, intermediate checkpoints, stagnation detection,
//! strict verification gating, hierarchical context summarisation, and the
//! budget-extension handshake.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

use crate::mode::Mode;
use crate::runs::budget::{Budget, BudgetVerdict};
use crate::runs::checkpoint_store::CheckpointStore;
use crate::runs::mode_policy::AgentModePolicy;
use crate::runs::stagnation::{classify_tool_call, StagnationDetector};
use crate::runs::state::{RunId, RunStatus, RunView};
use crate::runs::verification::VerificationLedger;
use mixd_providers::{
    AssistantToolCall, AssistantToolCallFn, Message, Provider, ProviderRequest, ReasoningControl,
    StreamEvent, ToolSpec, Usage,
};
use mixd_storage::Storage;
use mixd_tools::approval::ApprovalOutcome;
use mixd_tools::{ToolContext, ToolRegistry};

pub use super::events::{RunEvent, RunEventSink};

#[derive(Clone)]
pub struct RunEngine {
    pub storage: Arc<Storage>,
    pub providers: Arc<ProviderRegistry>,
    pub tools: Arc<tokio::sync::RwLock<ToolRegistry>>,
    pub executor: Arc<mixd_tools::execute::ToolExecutor>,
    pub checkpoints: Arc<CheckpointStore>,
    pub policy: AgentModePolicy,
    pub events: Arc<broadcast::Sender<RunEvent>>,
    pub sequences: Arc<tokio::sync::Mutex<HashMap<RunId, i64>>>,
    /// Streamed token events waiting to be written to `run_events` in a batch.
    ///
    /// `text` / `reasoning` arrive once per token; writing each one straight
    /// through made SQLite the bottleneck of every streamed turn (the WebView
    /// bridge was reporting `Lagged` receivers because of it). The buffer is
    /// drained on a timer, when it fills up, and — crucially — *before* any
    /// milestone event is persisted, so a Run that completes, fails or is
    /// cancelled always has its text on disk first.
    pub event_buffer: Arc<tokio::sync::Mutex<HashMap<RunId, Vec<RunEvent>>>>,
    /// Runs that already have a flush timer armed, so a timer is spawned at
    /// most once per buffering window instead of once per token.
    flush_scheduled: Arc<tokio::sync::Mutex<std::collections::HashSet<RunId>>>,
    pub active: Arc<tokio::sync::Mutex<HashMap<RunId, RunCancellation>>>,
    /// Budget-extension answers, delivered in memory by [`RunEngine::resolve_budget`].
    ///
    /// The database copy is kept for audit and for a resume across restarts;
    /// polling it was previously the only channel, which made the answer depend
    /// on a status transition that had never been persisted.
    pub budget_decisions: Arc<tokio::sync::Mutex<HashMap<RunId, bool>>>,
}

/// Cooperative cancellation handle for an in-flight Run.
#[derive(Clone)]
pub struct RunCancellation {
    pub flag: Arc<tokio::sync::Notify>,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl RunCancellation {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(tokio::sync::Notify::new()),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.flag.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Completes as soon as the Run is cancelled, or immediately if it already
    /// was. Await this in a `select!` alongside other blocking work.
    pub async fn cancelled_signal(&self) {
        let notified = self.flag.notified();
        tokio::pin!(notified);
        // Subscribe before re-checking, so a concurrent `cancel()` cannot slip
        // through the window between the check and the await.
        notified.as_mut().enable();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

impl Default for RunCancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl RunEngine {
    /// Build an engine bound to an existing storage handle. Used by the Tauri
    /// shell, which owns the app-data directory.
    pub fn with_storage(storage: Arc<Storage>, secrets: Arc<mixd_storage::SecretStore>) -> Self {
        let tools = ToolRegistry::new(mixd_tools::egress::EgressPolicy::default());
        Self::with_storage_and_tools(storage, secrets, tools)
    }

    /// Build an engine with a pre-populated Tool Registry (Browser Operator and
    /// MCP host attached by the app shell).
    pub fn with_storage_and_tools(
        storage: Arc<Storage>,
        secrets: Arc<mixd_storage::SecretStore>,
        tools: ToolRegistry,
    ) -> Self {
        let (tx, _) = broadcast::channel(4096);
        let executor = Arc::new(mixd_tools::execute::ToolExecutor::new(
            storage.clone(),
            secrets,
        ));
        Self {
            storage,
            providers: Arc::new(ProviderRegistry::default()),
            tools: Arc::new(tokio::sync::RwLock::new(tools)),
            executor,
            checkpoints: Arc::new(CheckpointStore::new(20)),
            policy: AgentModePolicy::default(),
            events: Arc::new(tx),
            sequences: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            event_buffer: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            flush_scheduled: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            active: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            budget_decisions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RunEvent> {
        self.events.subscribe()
    }

    /// Publish an event to live subscribers and to the replay log.
    ///
    /// Milestone events (`status`, `tool_*`, `todo_list`, `error`, `budget_*`,
    /// …) are written straight through. Streamed token events (`text`,
    /// `reasoning`) are buffered and flushed in batches: they are the ones that
    /// arrive once per token, and serialising a SQLite INSERT behind each of
    /// them made the database the bottleneck of a streaming turn.
    pub async fn emit(&self, run_id: RunId, kind: &str, payload: serde_json::Value) {
        let sequence = self.next_sequence(run_id).await;
        let evt = RunEvent {
            run_id,
            sequence,
            kind: kind.to_string(),
            data: payload,
            created_at: chrono::Utc::now(),
        };
        let _ = self.events.send(evt.clone());
        if STREAM_KINDS.contains(&kind) {
            self.buffer_event(evt).await;
            return;
        }
        // A Run's terminal state is itself a milestone, so draining the token
        // buffer here is what guarantees the streamed text is durable the
        // moment a Run completes, fails or is cancelled — before the row that
        // says so is written.
        self.flush_events(run_id).await;
        if let Err(err) = self.persist_event(&evt).await {
            tracing::error!(%run_id, sequence, kind, %err, "persisting run event failed");
        }
    }

    /// Park a token-stream event until the next batched flush.
    async fn buffer_event(&self, evt: RunEvent) {
        let run_id = evt.run_id;
        let overflow = {
            let mut buffer = self.event_buffer.lock().await;
            let slot = buffer.entry(run_id).or_default();
            slot.push(evt);
            slot.len() >= EVENT_FLUSH_BATCH
        };
        if overflow {
            self.flush_events(run_id).await;
            return;
        }
        // Arm at most one timer per buffering window. Disarming happens in the
        // task *before* it drains, so an event buffered in that gap arms a new
        // timer instead of being left with no flush scheduled behind it.
        let mut scheduled = self.flush_scheduled.lock().await;
        if scheduled.insert(run_id) {
            let engine = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(EVENT_FLUSH_INTERVAL).await;
                engine.flush_scheduled.lock().await.remove(&run_id);
                engine.flush_events(run_id).await;
            });
        }
    }

    /// Write every buffered event of `run_id` to `run_events`, oldest first.
    pub async fn flush_events(&self, run_id: RunId) {
        let batch = {
            let mut buffer = self.event_buffer.lock().await;
            buffer.remove(&run_id).unwrap_or_default()
        };
        for evt in batch {
            if let Err(err) = self.persist_event(&evt).await {
                tracing::error!(
                    %run_id,
                    sequence = evt.sequence,
                    kind = %evt.kind,
                    %err,
                    "persisting buffered run event failed"
                );
            }
        }
    }

    /// Drain every Run's buffer. Used before a replay so reloading a
    /// conversation can never show a transcript the log has not caught up on.
    pub async fn flush_all_events(&self) {
        let all = {
            let mut buffer = self.event_buffer.lock().await;
            std::mem::take(&mut *buffer)
        };
        for (run_id, batch) in all {
            for evt in batch {
                if let Err(err) = self.persist_event(&evt).await {
                    tracing::error!(
                        %run_id,
                        sequence = evt.sequence,
                        kind = %evt.kind,
                        %err,
                        "persisting buffered run event failed"
                    );
                }
            }
        }
    }

    /// Reserve the next sequence number for a Run.
    ///
    /// The lock only ever covers a counter increment. The database lookup that
    /// seeds the counter (a Run resumed after a restart has to continue past
    /// the rows already stored) happens outside it: holding it across the
    /// SQLite round trip would serialise every TextDelta of a streaming turn
    /// behind one query, which is exactly the bottleneck this design removes.
    async fn next_sequence(&self, run_id: RunId) -> i64 {
        {
            let mut seqs = self.sequences.lock().await;
            if let Some(entry) = seqs.get_mut(&run_id) {
                *entry += 1;
                return *entry;
            }
        }
        let base = self.persisted_sequence_base(run_id).await;
        let mut seqs = self.sequences.lock().await;
        // Another emitter can seed the counter while the query runs. `or_insert`
        // means only one of them wins, and taking the maximum keeps a *failed*
        // read (which falls back to 0) from walking the counter backwards into
        // rows that already exist — that, not concurrency, is the only way a
        // sequence could collide with `UNIQUE(run_id, sequence)`.
        let entry = seqs.entry(run_id).or_insert(base);
        *entry = (*entry).max(base);
        *entry += 1;
        *entry
    }

    /// Highest sequence already stored for a Run, or 0 when there is none.
    ///
    /// Retried once because a transient "database is locked" here used to be
    /// silently treated as "no rows", which would then collide on insert.
    async fn persisted_sequence_base(&self, run_id: RunId) -> i64 {
        let mut last: Option<sqlx::Error> = None;
        for _ in 0..2 {
            match sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(sequence), 0) FROM run_events WHERE run_id = ?",
            )
            .bind(run_id.to_string())
            .fetch_optional(self.storage.pool())
            .await
            {
                Ok(Some(base)) => return base,
                Ok(None) => return 0,
                Err(err) => last = Some(err),
            }
        }
        if let Some(err) = last {
            tracing::error!(%run_id, %err, "reading the run event high-water mark failed");
        }
        0
    }

    async fn persist_event(&self, evt: &RunEvent) -> anyhow::Result<()> {
        if let Err(err) = self.insert_event(evt).await {
            if !is_unique_violation(&err) {
                return Err(err);
            }
            // Only reachable when the high-water mark read failed and the
            // counter was seeded too low. `run_events` has
            // UNIQUE(run_id, sequence) and replay orders by that column, so
            // the row is renumbered against a fresh read rather than dropped:
            // dropping it would punch a hole in the transcript. The broadcast
            // envelope keeps the sequence it was published with, which only
            // affects gap detection, never replay order.
            let base = self.persisted_sequence_base(evt.run_id).await;
            let renumbered = base + 1;
            {
                let mut seqs = self.sequences.lock().await;
                let entry = seqs.entry(evt.run_id).or_insert(renumbered);
                *entry = (*entry).max(renumbered);
            }
            let stored = RunEvent {
                sequence: renumbered,
                ..evt.clone()
            };
            tracing::warn!(
                run = %evt.run_id,
                sequence = evt.sequence,
                renumbered,
                "run event sequence collided with a persisted row; renumbering"
            );
            self.insert_event(&stored).await
        } else {
            Ok(())
        }
    }

    async fn insert_event(&self, evt: &RunEvent) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO run_events (id, run_id, sequence, kind, data) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(evt.run_id.to_string())
        .bind(evt.sequence)
        .bind(&evt.kind)
        .bind(evt.data.to_string())
        .execute(self.storage.pool())
        .await?;
        Ok(())
    }

    pub async fn list_providers(&self) -> Vec<ProviderHandle> {
        self.providers.list().await
    }

    /// Replace the Tool Registry (used after an MCP reconnect).
    pub async fn replace_tools(&self, tools: ToolRegistry) {
        *self.tools.write().await = tools;
    }

    /// Advertised Tool specs, including Browser and MCP families.
    pub async fn tool_specs(&self) -> Vec<mixd_providers::ToolSpec> {
        self.tools
            .read()
            .await
            .specs()
            .into_iter()
            .map(|s| mixd_providers::ToolSpec {
                name: s.name,
                description: s.description,
                parameters: s.parameters,
            })
            .collect()
    }

    pub async fn cancel(&self, run_id: RunId) {
        let map = self.active.lock().await;
        if let Some(handle) = map.get(&run_id) {
            handle.cancel();
        }
    }

    pub async fn run_state(&self, id: RunId) -> Option<RunView> {
        let row = sqlx::query("SELECT id, conversation_id, data FROM runs WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.storage.pool())
            .await
            .ok()
            .flatten()?;
        run_view_from_row(&row)
    }

    /// All Runs of a conversation, oldest first — the persisted counterpart of
    /// the live event stream, used to rebuild tool activity for past turns.
    pub async fn list_runs(&self, conversation_id: uuid::Uuid) -> Vec<RunView> {
        let rows = sqlx::query(
            "SELECT id, conversation_id, data FROM runs \
             WHERE conversation_id = ? ORDER BY created_at ASC",
        )
        .bind(conversation_id.to_string())
        .fetch_all(self.storage.pool())
        .await
        .unwrap_or_default();
        rows.iter().filter_map(run_view_from_row).collect()
    }

    /// Record a status transition for a Run, both in the denormalised `status`
    /// column (what the UI lists) and inside the `data` blob (what
    /// [`run_view_from_row`] reads).
    ///
    /// Every value is bound as a parameter: paths and JSON payloads were
    /// previously interpolated into the SQL text, which SQLite rejected, and
    /// the resulting error was discarded — so no transition was ever persisted.
    async fn set_run_state(&self, run_id: RunId, status: RunStatus, extra: serde_json::Value) {
        let mut sql =
            String::from("UPDATE runs SET data = json_set(data, '$.status', ?, '$.updatedAt', ?");
        let mut binds: Vec<String> =
            vec![status.as_str().to_string(), chrono::Utc::now().to_rfc3339()];
        for (k, v) in extra.as_object().into_iter().flatten() {
            sql.push_str(", ?, json(?)");
            binds.push(format!(
                "$.\"{}\"",
                k.replace('\\', "\\\\").replace('"', "\\\"")
            ));
            binds.push(v.to_string());
        }
        sql.push_str("), status = ? WHERE id = ?");

        let mut query = sqlx::query(&sql);
        for b in &binds {
            query = query.bind(b);
        }
        let result = query
            .bind(status.as_str())
            .bind(run_id.to_string())
            .execute(self.storage.pool())
            .await;
        match result {
            Ok(res) if res.rows_affected() == 0 => {
                tracing::error!(
                    %run_id,
                    status = status.as_str(),
                    "set_run_state matched no row; status was not persisted"
                );
            }
            Ok(_) => {}
            Err(err) => {
                tracing::error!(%run_id, status = status.as_str(), %err, "set_run_state failed");
            }
        }
    }

    /// Drop the cached skill and agent catalogues.
    ///
    /// Both are read once and reused for the process, so a profile the user just
    /// saved would otherwise stay invisible until the app was restarted.
    pub async fn invalidate_catalogues(&self) {
        self.tools.read().await.invalidate_catalogues().await;
    }

    /// Append one turn to the conversation transcript.
    ///
    /// The transcript is what a later turn's history is rebuilt from, so the
    /// engine writes it here rather than leaving it to the caller: the engine is
    /// the only place that knows the correct interleaving of assistant tool-call
    /// turns and their results. Persisting only the final assistant text — which
    /// is what the shell did — meant a conversation's second turn contained no
    /// trace of the first turn's tool use, so the model re-derived the same
    /// search and edit work from scratch.
    async fn append_message(
        &self,
        conversation_id: uuid::Uuid,
        run_id: RunId,
        model_id: &str,
        message: &HistoryMessage,
    ) {
        let data = serde_json::json!({
            "role": message.role,
            "content": message.content,
            "modelId": model_id,
            "runId": run_id.to_string(),
            "toolCalls": message.tool_calls,
            "toolCallId": message.tool_call_id,
            "name": message.name,
            "isError": message.is_error,
            "kind": message.role,
            "createdAt": chrono::Utc::now().to_rfc3339(),
        });
        if let Err(err) = sqlx::query(
            "INSERT INTO messages (id, owner_id, data, conversation_id) VALUES (?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(data.to_string())
        .bind(conversation_id.to_string())
        .execute(self.storage.pool())
        .await
        {
            tracing::warn!(?err, "failed to persist transcript message");
        }
    }
}

fn parse_status(value: &str) -> Option<RunStatus> {
    match value {
        "queued" => Some(RunStatus::Queued),
        "running" => Some(RunStatus::Running),
        "waiting_approval" => Some(RunStatus::WaitingApproval),
        "budget_extension_pending" => Some(RunStatus::BudgetExtensionPending),
        "interrupted" => Some(RunStatus::Interrupted),
        "failed" => Some(RunStatus::Failed),
        "cancelled" => Some(RunStatus::Cancelled),
        "completed" => Some(RunStatus::Completed),
        _ => None,
    }
}

/// Map a `runs` row (`id`, `conversation_id`, `data`) onto the public
/// [`RunView`] shape. Rows with an unparseable id are skipped by the caller.
fn run_view_from_row(row: &sqlx::sqlite::SqliteRow) -> Option<RunView> {
    use sqlx::Row;
    let id = uuid::Uuid::parse_str(&row.try_get::<String, _>("id").ok()?).ok()?;
    let conversation_id = row
        .try_get::<String, _>("conversation_id")
        .ok()
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())?;
    let data: serde_json::Value =
        serde_json::from_str(&row.try_get::<String, _>("data").ok()?).unwrap_or_default();
    let status = data
        .get("status")
        .and_then(|v| v.as_str())
        .and_then(parse_status)
        .unwrap_or(RunStatus::Queued);
    Some(RunView {
        id,
        conversation_id,
        mode: data
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("chat")
            .to_string(),
        status,
        model_id: data
            .get("modelId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        step_count: data.get("stepCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        tool_call_count: data
            .get("toolCallCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        started_at: data
            .get("startedAt")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now),
        updated_at: data
            .get("updatedAt")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now),
        summary: data
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        budget: data.get("budget").cloned().unwrap_or_default(),
    })
}

/// Registry of currently configured LLM providers.
#[derive(Default)]
pub struct ProviderRegistry {
    pub inner: tokio::sync::RwLock<HashMap<String, Arc<dyn Provider>>>,
}

impl ProviderRegistry {
    pub async fn register(&self, key: String, provider: Arc<dyn Provider>) {
        self.inner.write().await.insert(key, provider);
    }

    pub async fn unregister(&self, key: &str) {
        self.inner.write().await.remove(key);
    }

    pub async fn get(&self, key: &str) -> Option<Arc<dyn Provider>> {
        self.inner.read().await.get(key).cloned()
    }

    pub async fn list(&self) -> Vec<ProviderHandle> {
        let guard = self.inner.read().await;
        let mut out: Vec<ProviderHandle> = guard
            .iter()
            .map(|(id, p)| ProviderHandle {
                id: id.clone(),
                label: p.label().to_string(),
                capabilities: p.capabilities(),
            })
            .collect();
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }
}

#[derive(Debug, Clone)]
pub struct ProviderHandle {
    pub id: String,
    pub label: String,
    pub capabilities: mixd_providers::ProviderCapabilities,
}

/// How many times the agent may be sent back to verify an unconfirmed
/// destructive call before the run concludes anyway.
const MAX_UNVERIFIED_REPAIR_TURNS: u32 = 2;

/// Event kinds that carry a single token of streamed output. Buffered rather
/// than written one row at a time — see [`RunEngine::emit`].
const STREAM_KINDS: &[&str] = &["text", "reasoning"];

/// How long a buffered token event may wait before it is written anyway, so a
/// Run that stops emitting milestones still becomes durable promptly.
const EVENT_FLUSH_INTERVAL: Duration = Duration::from_millis(250);

/// Flush as soon as the buffer reaches this many events, so a burst of deltas
/// cannot grow it without bound while the timer is still pending.
const EVENT_FLUSH_BATCH: usize = 64;

/// Did the insert fail on `run_events.UNIQUE(run_id, sequence)`?
fn is_unique_violation(err: &anyhow::Error) -> bool {
    err.downcast_ref::<sqlx::Error>()
        .and_then(|e| e.as_database_error())
        .map(|e| e.message().contains("UNIQUE constraint failed"))
        .unwrap_or(false)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// Must survive persistence. Without it, reloading a conversation produced
    /// `tool` messages whose originating assistant turn had no `tool_calls`,
    /// which every provider rejects.
    #[serde(default)]
    pub tool_calls: Vec<AssistantToolCall>,
    /// Whether this `tool` message reported a failure. Adapters translate it
    /// into their own error channel, so losing it on reload changes the model's
    /// view of what went wrong.
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug)]
pub struct RunRequest {
    pub run_id: RunId,
    pub conversation_id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    /// Absolute folder this run is scoped to (project root). `None` keeps the
    /// legacy behaviour of resolving against the process working directory.
    pub root_path: Option<String>,
    pub mode: Mode,
    pub model_id: String,
    pub provider_id: String,
    pub system: Option<String>,
    pub temperature: Option<f32>,
    pub reasoning: Option<ReasoningControl>,
    pub max_tokens: Option<u32>,
    pub tools: Vec<ToolSpec>,
    pub history: Vec<HistoryMessage>,
    pub user_message: String,
    pub resume_from: Option<CheckpointSnapshot>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointSnapshot {
    pub step_index: u32,
    pub messages: Vec<HistoryMessage>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RunOutcome {
    pub status: RunStatus,
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<RunToolCall>,
    pub usage: Option<Usage>,
    pub finish_reason: Option<String>,
    pub steps: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub approval_id: Option<String>,
}

impl RunEngine {
    /// The Agent loop. Runs at least one Provider turn; keeps looping while the
    /// model requests Tools and the budget allows it.
    ///
    /// `run_id` is the only field worth indexing a log line by, and `self`
    /// carries the storage handles, so both stay out of the span.
    #[tracing::instrument(skip_all, fields(run_id = %request.run_id))]
    pub async fn execute(self: &Arc<Self>, request: RunRequest) -> anyhow::Result<RunOutcome> {
        let run_id = request.run_id;
        let provider = self
            .providers
            .get(&request.provider_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("provider not configured: {}", request.provider_id))?;

        let cancellation = RunCancellation::new();
        self.active
            .lock()
            .await
            .insert(run_id, cancellation.clone());

        let mut budget = Budget::for_mode(request.mode);
        // The stored budget is an override *on top of* the mode's, not a
        // replacement for it: the shell writes `null` for any limit the user did
        // not edit, and treating that as the global default would shrink an
        // agent run from 300 steps to 12 the moment anyone granted an extension.
        if let Some(stored) = budget_snapshot(&run_id, self).await {
            let base = budget.clone();
            budget = Budget::from_json_with_base(&stored, base);
        }

        let mut messages: Vec<Message> = request
            .resume_from
            .as_ref()
            .map(|s| s.messages.iter().map(history_to_message).collect())
            .unwrap_or_default();
        if request.resume_from.is_none() {
            for h in &request.history {
                messages.push(history_to_message(h));
            }
            messages.push(Message::text("user", request.user_message.clone()));
            self.emit(
                run_id,
                "message",
                serde_json::json!({ "role": "user", "content": request.user_message }),
            )
            .await;

            // Inject relevant Memory / Knowledge once, at the head of the Run.
            if let Ok(block) =
                crate::retrieval::retrieve_all(self.storage.as_ref(), &request.user_message).await
            {
                if let Some(msg) = block.to_message() {
                    self.emit(
                        run_id,
                        "context_summary",
                        serde_json::json!({
                            "memories": block.memories.len(),
                            "knowledge": block.knowledge.len(),
                        }),
                    )
                    .await;
                    messages.insert(0, msg);
                }
            }
        }

        self.set_run_state(run_id, RunStatus::Running, serde_json::json!({}))
            .await;
        self.emit(
            run_id,
            "status",
            serde_json::json!({ "status": "running", "mode": request.mode.as_str() }),
        )
        .await;
        self.emit(
            run_id,
            "model_selected",
            serde_json::json!({ "model_id": request.model_id, "provider_id": request.provider_id }),
        )
        .await;

        let mut steps: u32 = 0;
        let mut total_tool_calls: u32 = 0;
        let mut all_tool_calls: Vec<RunToolCall> = Vec::new();
        let mut final_text = String::new();
        let mut final_reasoning = String::new();
        let mut last_usage: Option<Usage> = None;
        let mut last_finish: Option<String> = None;
        let mut stagnation = StagnationDetector::new();
        let mut verification = VerificationLedger::default();
        // Signals for the current turn, collected as tool calls execute. Kept
        // out here because the classifier needs the tool result, which only
        // exists after dispatch.
        let mut signals: Vec<crate::runs::stagnation::TurnSignal> = Vec::new();
        // How many extra turns the agent has been given to verify an
        // unconfirmed destructive call. Bounded so a model that never reads
        // anything back cannot loop forever.
        let mut unverified_repair_turns: u32 = 0;
        // Last checklist emitted, so the UI only gets an event when it changes.
        let mut last_todos: Vec<mixd_tools::todo::TodoItem> = Vec::new();
        let mut summary = crate::context::TieredSummary::new();
        let task_state = crate::context::TaskState {
            goal: request.user_message.chars().take(200).collect(),
            ..Default::default()
        };
        // The skill catalogue belongs in the system prompt, not in the history:
        // it is the same on every turn and the adapters cache it there.
        let mut system = request.system.clone().unwrap_or_default();
        let skills_prompt = self.tools.read().await.skills_prompt().await;
        if !skills_prompt.trim().is_empty() {
            if !system.trim().is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(&skills_prompt);
        }
        if !request.tools.is_empty() {
            let allowed: Vec<&str> = request.tools.iter().map(|t| t.name.as_str()).collect();
            system.push_str(&format!(
                "\n<tool_policy>\nFor this run you may call only: {}. Anything else will be \
                 rejected.\n</tool_policy>",
                allowed.join(", ")
            ));
        }

        loop {
            if cancellation.is_cancelled() {
                self.set_run_state(run_id, RunStatus::Cancelled, serde_json::json!({}))
                    .await;
                self.emit(
                    run_id,
                    "status",
                    serde_json::json!({ "status": "cancelled" }),
                )
                .await;
                return Ok(RunOutcome {
                    status: RunStatus::Cancelled,
                    text: final_text,
                    reasoning: final_reasoning,
                    tool_calls: all_tool_calls,
                    usage: last_usage,
                    finish_reason: last_finish,
                    steps,
                });
            }

            // Budget gate before each Provider turn.
            match budget.check(steps, total_tool_calls).await {
                BudgetVerdict::Ok => {}
                BudgetVerdict::Exhausted(reason) => {
                    let grant = self.await_budget_extension(run_id, reason, &budget).await;
                    // A granted extension can still be refused by the grant
                    // allowance, so the two outcomes are distinguished rather
                    // than both reported as "declined".
                    let extended = grant && budget.extend();
                    let stop_reason = if !grant {
                        "budget_declined"
                    } else {
                        "budget_grants_exhausted"
                    };
                    if extended {
                        self.set_run_state(run_id, RunStatus::Running, serde_json::json!({}))
                            .await;
                    } else {
                        self.set_run_state(run_id, RunStatus::Completed, serde_json::json!({}))
                            .await;
                        self.emit(
                            run_id,
                            "status",
                            serde_json::json!({ "status": "completed", "reason": stop_reason }),
                        )
                        .await;
                        return Ok(RunOutcome {
                            status: RunStatus::Completed,
                            text: final_text,
                            reasoning: final_reasoning,
                            tool_calls: all_tool_calls,
                            usage: last_usage,
                            finish_reason: last_finish,
                            steps,
                        });
                    }
                }
            }

            steps += 1;

            // Hierarchical context compression. The head block — the task goal
            // plus the agent's own checklist — is prepended so both survive the
            // folding of older turns. Without the checklist an agent loses track
            // of a five-step refactor as soon as the context compresses.
            if self.policy.context_tiered_summary {
                let head = render_head(&task_state, &self.tools.read().await.todos).await;
                let current = if head.trim().is_empty() {
                    messages.clone()
                } else {
                    let mut v = vec![Message::text("system", head)];
                    v.extend(messages.iter().cloned());
                    v
                };
                let owned = current.clone();
                let model = request.model_id.clone();
                let provider_handle = provider.clone();
                // The summariser is handed exactly the slice being dropped, so
                // the summary never duplicates the tail kept verbatim.
                let outcome =
                    crate::context::compress(&current, summary.clone(), |older| async move {
                        summarize_turns(provider_handle.as_ref(), &model, &older).await
                    })
                    .await;
                if !outcome.skipped {
                    summary = outcome.summary.clone();
                    messages = strip_head(outcome.messages);
                    self.emit(
                        run_id,
                        "context_summary",
                        serde_json::json!({
                            "compressed": true,
                            "activeChars": summary.char_count(),
                            "archiveRows": summary.archive.len(),
                            "messages": messages.len(),
                            "sourceMessages": owned.len(),
                        }),
                    )
                    .await;
                }
            }

            // The head block is stripped again by `strip_head` after compression,
            // so it has to be re-attached here even when compression was skipped;
            // otherwise the goal and the checklist vanish after step one.
            // Surface the checklist to the UI, not just to the model: a plan the
            // user cannot see is a plan they cannot correct.
            let checklist = self.tools.read().await.todos.snapshot().await;
            if checklist != last_todos {
                last_todos = checklist.clone();
                self.emit(
                    run_id,
                    "todo_list",
                    serde_json::json!({
                        "todos": checklist
                            .iter()
                            .map(|item| serde_json::json!({
                                "id": item.id,
                                "content": item.content,
                                "status": item.status,
                            }))
                            .collect::<Vec<_>>(),
                    }),
                )
                .await;
            }

            let head = if self.policy.context_tiered_summary {
                render_head(&task_state, &self.tools.read().await.todos).await
            } else {
                String::new()
            };
            let outgoing = if head.trim().is_empty() {
                messages.clone()
            } else {
                let mut v = vec![Message::text("system", head)];
                v.extend(messages.iter().cloned());
                v
            };

            let turn = self
                .run_turn(run_id, provider.as_ref(), &request, &outgoing, &system)
                .await?;

            if let Some(err) = turn.error {
                self.emit(run_id, "error", serde_json::json!({ "error": err }))
                    .await;
                self.set_run_state(
                    run_id,
                    RunStatus::Failed,
                    serde_json::json!({ "error": err }),
                )
                .await;
                self.emit(
                    run_id,
                    "status",
                    serde_json::json!({ "status": "failed", "error": err }),
                )
                .await;
                return Ok(RunOutcome {
                    status: RunStatus::Failed,
                    text: final_text,
                    reasoning: final_reasoning,
                    tool_calls: all_tool_calls,
                    usage: last_usage,
                    finish_reason: last_finish,
                    steps,
                });
            }

            last_usage = turn.usage.clone();
            last_finish = turn.finish_reason.clone();
            if !turn.reasoning.is_empty() {
                final_reasoning = turn.reasoning.clone();
            }

            // Stagnation detection: inject a repair hint as a user-role turn.
            //
            // A turn that produced no tool calls and no error is progress, and
            // the loop ends below; anything else is fed in as-is.
            if turn.tool_calls.is_empty() && turn.error.is_none() {
                signals.clear();
                signals.push(crate::runs::stagnation::TurnSignal::Progress);
            }
            if self.policy.stagnation_detection {
                if let Some(report) = stagnation.observe(signals.clone()) {
                    self.emit(
                        run_id,
                        "stagnation_detected",
                        serde_json::json!({ "hint": report.hint, "kind": report.kind }),
                    )
                    .await;
                    messages.push(Message::text(
                        "user",
                        format!(
                            "[self-repair hint] {} — adjust the approach before retrying.",
                            report.hint
                        ),
                    ));
                }
            }

            if turn.tool_calls.is_empty() {
                // Strict verification, enforced. A destructive call that was
                // never read back leaves the ledger open, and the model is not
                // allowed to conclude: it gets one more turn to verify the
                // effect. Previously the ledger was written and never read, so
                // this branch could not fire.
                if self.policy.strict_verification && verification.is_blocked() {
                    if unverified_repair_turns >= MAX_UNVERIFIED_REPAIR_TURNS {
                        self.emit(
                            run_id,
                            "verification_failed",
                            serde_json::json!({
                                "outstanding": verification.outstanding(),
                                "reason": verification.last_reason(),
                            }),
                        )
                        .await;
                        final_text = turn.text.clone();
                        messages.push(Message::text("assistant", turn.text.clone()));
                        if !turn.text.is_empty() {
                            self.emit(
                                run_id,
                                "message",
                                serde_json::json!({
                                    "role": "assistant",
                                    "content": turn.text,
                                    "model_id": request.model_id,
                                }),
                            )
                            .await;
                        }
                        break;
                    }
                    unverified_repair_turns += 1;
                    let outstanding = verification.outstanding();
                    self.emit(
                        run_id,
                        "verification_required",
                        serde_json::json!({
                            "tool": outstanding.first().cloned().unwrap_or_default(),
                            "outstanding": outstanding,
                            "reason": verification.last_reason(),
                        }),
                    )
                    .await;
                    messages.push(Message::text(
                        "user",
                        format!(
                            "[verification] You have not confirmed the effect of {}. \
                             Read the affected path back and state what is actually there \
                             before you conclude. Outstanding: {}.",
                            outstanding.join(", "),
                            verification
                                .last_reason()
                                .unwrap_or_else(|| "unconfirmed change".to_string())
                        ),
                    ));
                    continue;
                }
                final_text = turn.text.clone();
                let final_message = Message::text("assistant", turn.text.clone());
                self.append_message(
                    request.conversation_id,
                    run_id,
                    &request.model_id,
                    &message_to_history(&final_message),
                )
                .await;
                messages.push(final_message);
                if !turn.text.is_empty() {
                    self.emit(
                        run_id,
                        "message",
                        serde_json::json!({
                            "role": "assistant",
                            "content": turn.text,
                            "reasoning": turn.reasoning,
                            "model_id": request.model_id,
                        }),
                    )
                    .await;
                }
                break;
            }

            // Assistant turn requested Tools — record it and execute them.
            let assistant_message =
                Message::assistant_with_tools(turn.text.clone(), turn.tool_calls.clone());
            self.append_message(
                request.conversation_id,
                run_id,
                &request.model_id,
                &message_to_history(&assistant_message),
            )
            .await;
            messages.push(assistant_message);
            if !turn.text.is_empty() {
                self.emit(
                    run_id,
                    "message",
                    serde_json::json!({ "role": "assistant", "content": turn.text, "partial": true }),
                )
                .await;
            }

            let mut tool_messages: Vec<Message> = Vec::new();
            for call in &turn.tool_calls {
                total_tool_calls += 1;
                if crate::runs::stagnation::is_read_only(call)
                    && stagnation.note_read(crate::runs::stagnation::read_key(call))
                {
                    signals.push(crate::runs::stagnation::TurnSignal::RepeatedRead);
                }
                verification.confirm(&call.function.name);
                let result = self.dispatch_tool(run_id, &request, call).await;
                all_tool_calls.push(result.clone());
                let is_error = result.error.is_some();
                let content = match &result.error {
                    Some(err) => format!("Error: {}", err),
                    None => result
                        .result
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "ok".to_string()),
                };
                // The failure flag is what the adapters turn into their own error
                // channel and what the stagnation detector reads, so it has to be
                // set here rather than inferred from the message text later.
                signals.push(classify_tool_call(call, is_error));
                tool_messages.push(Message::tool_result(call, content, is_error));
            }
            for message in &tool_messages {
                self.append_message(
                    request.conversation_id,
                    run_id,
                    &request.model_id,
                    &message_to_history(message),
                )
                .await;
            }
            messages.extend(tool_messages);

            // Strict verification: note the destructive calls in this turn so
            // the concluding turn above knows they need a confirming read.
            if self.policy.strict_verification {
                for call in &turn.tool_calls {
                    let name = call.function.name.as_str();
                    let args: serde_json::Value =
                        serde_json::from_str(&call.function.arguments).unwrap_or_default();
                    if verification.requires_confirmation(name, &args) {
                        self.emit(
                            run_id,
                            "destructive_tool_called",
                            serde_json::json!({
                                "tool": name,
                                "arguments": args,
                                "reason": verification.last_reason().unwrap_or_default(),
                            }),
                        )
                        .await;
                    }
                }
            }

            // Intermediate checkpointing.
            if self.policy.checkpointing && steps % self.policy.checkpoint_every_steps == 0 {
                let snapshot = CheckpointSnapshot {
                    step_index: steps,
                    messages: messages.iter().map(message_to_history).collect(),
                };
                let id = self
                    .checkpoints
                    .save(
                        run_id,
                        steps,
                        total_tool_calls,
                        &snapshot,
                        self.storage.as_ref(),
                    )
                    .await;
                self.emit(
                    run_id,
                    "checkpoint_saved",
                    serde_json::json!({ "checkpoint_id": id, "step": steps }),
                )
                .await;
            }
        }

        self.set_run_state(
            run_id,
            RunStatus::Completed,
            serde_json::json!({
                "stepCount": steps,
                "toolCallCount": total_tool_calls,
            }),
        )
        .await;
        self.active.lock().await.remove(&run_id);
        self.emit(
            run_id,
            "status",
            serde_json::json!({
                "status": "completed",
                "steps": steps,
                "tool_calls": total_tool_calls,
                "usage": last_usage,
            }),
        )
        .await;

        Ok(RunOutcome {
            status: RunStatus::Completed,
            text: final_text,
            reasoning: final_reasoning,
            tool_calls: all_tool_calls,
            usage: last_usage,
            finish_reason: last_finish,
            steps,
        })
    }

    /// Ask the user for a budget extension. Returns `true` when granted.
    async fn await_budget_extension(&self, run_id: RunId, reason: &str, budget: &Budget) -> bool {
        self.set_run_state(
            run_id,
            RunStatus::BudgetExtensionPending,
            serde_json::json!({}),
        )
        .await;
        self.emit(
            run_id,
            "budget_extension_requested",
            serde_json::json!({
                "reason": reason,
                "stepLimit": budget.step_limit,
                "toolCallLimit": budget.tool_call_limit,
                "wallClockMinutes": budget.wall_clock_minutes,
                "grants": budget.grants_used + 1,
            }),
        )
        .await;
        // Wait for the UI to answer via `budget_extension`.
        let deadline =
            std::time::Instant::now() + Duration::from_secs(budget.extension_timeout_secs);
        let granted = loop {
            if let Some(granted) = self.take_budget_decision(run_id).await {
                break granted;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        self.budget_decisions.lock().await.remove(&run_id);
        granted
    }

    /// Record the user's answer to a pending budget-extension request.
    ///
    /// Returns an error when the Run is not (or no longer) waiting for one, so
    /// the UI never treats an ignored answer as accepted. `budget` carries the
    /// raised limits a grant applies; they are persisted so a later resume sees
    /// the same numbers.
    pub async fn resolve_budget(
        &self,
        run_id: RunId,
        granted: bool,
        budget: Option<serde_json::Value>,
    ) -> Result<(), String> {
        let mut sql = if granted {
            String::from(
                "UPDATE runs \
                 SET data = json_set(data, '$.budgetDecision', json('true')",
            )
        } else {
            String::from(
                "UPDATE runs \
                 SET data = json_set(data, '$.budgetDecision', json('false')",
            )
        };
        let mut binds: Vec<String> = Vec::new();
        if granted {
            if let Some(budget) = &budget {
                sql.push_str(", '$.budget', json(?)");
                binds.push(budget.to_string());
            }
            sql.push_str("), status = 'running'");
        } else {
            sql.push_str("), status = 'completed'");
        }
        sql.push_str(" WHERE id = ? AND status = 'budget_extension_pending'");

        let mut query = sqlx::query(&sql);
        for b in &binds {
            query = query.bind(b);
        }
        let res = query
            .bind(run_id.to_string())
            .execute(self.storage.pool())
            .await
            .map_err(|e| e.to_string())?;
        if res.rows_affected() == 0 {
            return Err(format!(
                "run {run_id} is not waiting for a budget extension"
            ));
        }
        self.budget_decisions.lock().await.insert(run_id, granted);
        Ok(())
    }

    async fn take_budget_decision(&self, run_id: RunId) -> Option<bool> {
        if let Some(granted) = self.budget_decisions.lock().await.get(&run_id).copied() {
            return Some(granted);
        }
        let row: Option<(String,)> =
            sqlx::query_as::<_, (String,)>("SELECT data FROM runs WHERE id = ?")
                .bind(run_id.to_string())
                .fetch_optional(self.storage.pool())
                .await
                .unwrap_or(None);
        let data: serde_json::Value = row
            .and_then(|(raw,)| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        data.get("budgetDecision").and_then(|v| v.as_bool())
    }

    /// Dispatch a single Tool call, honouring the permission policy.
    #[tracing::instrument(skip_all, fields(run_id = %run_id))]
    async fn dispatch_tool(
        &self,
        run_id: RunId,
        request: &RunRequest,
        call: &AssistantToolCall,
    ) -> RunToolCall {
        let arguments: serde_json::Value =
            serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::Value::Null);
        let tool_name = call.function.name.clone();
        let tool_call_id = call.id.clone();

        let mut record = RunToolCall {
            id: tool_call_id.clone(),
            name: tool_name.clone(),
            arguments: arguments.clone(),
            result: None,
            error: None,
            approval_id: None,
        };

        // Permission gate: a stored rule may allow, deny, or prompt.
        //
        // The fallback is per-*call*, not per tool: `files` is one tool but its
        // read and write actions warrant different answers, and an unknown tool
        // is always treated as sensitive rather than optimistically allowed.
        let default_decision = {
            let registry = self.tools.read().await;
            match registry.get(&tool_name) {
                Some(tool) => tool.decision_for(&arguments),
                None => mixd_tools::permission::PermissionDecision::Ask,
            }
        };
        // A stored rule still wins, so a user can lock a read-only tool down or
        // silence a prompt for a mutating one.
        let decision = self
            .executor
            .resolve_permission(&tool_name, default_decision)
            .await;
        if decision == mixd_tools::permission::PermissionDecision::Deny {
            return self
                .deny_tool(run_id, record, "permission denied by rule")
                .await;
        }

        let mut approval_token: Option<String> = None;
        if decision == mixd_tools::permission::PermissionDecision::Ask {
            // `approvals.tool_call_id` references `tool_calls(id)`, so the call
            // row has to exist before the prompt can be persisted.
            let _ = self
                .executor
                .record_tool_call(
                    run_id,
                    &tool_call_id,
                    &tool_name,
                    arguments.clone(),
                    "awaiting_approval",
                    None,
                    None,
                )
                .await;
            let prompt = self
                .executor
                .request_approval(run_id, &tool_call_id, &tool_name, arguments.clone())
                .await;
            let Ok((rx, expires_at)) = prompt else {
                return self
                    .deny_tool(run_id, record, "could not open approval prompt")
                    .await;
            };
            let approval = self
                .executor
                .pending
                .lock()
                .await
                .iter()
                .find(|p| p.tool_call_id == tool_call_id)
                .map(|p| p.approval_id.clone());
            record.approval_id = approval.clone();
            approval_token = approval;

            self.set_run_state(
                run_id,
                RunStatus::WaitingApproval,
                serde_json::json!({ "approvalId": record.approval_id }),
            )
            .await;
            self.emit(
                run_id,
                "tool_approval_requested",
                serde_json::json!({
                    "tool_call_id": tool_call_id,
                    "approval_id": record.approval_id,
                    "name": tool_name,
                    "arguments": arguments,
                }),
            )
            .await;

            // Wait for the user's answer, the prompt TTL, or cancellation.
            let cancellation = self.cancellation_of(run_id).await;
            let ttl = (expires_at - chrono::Utc::now())
                .to_std()
                .unwrap_or(std::time::Duration::from_secs(0));
            let outcome = tokio::select! {
                biased;
                _ = cancellation.cancelled_signal() => ApprovalOutcome::Deny,
                Ok(outcome) = rx => outcome,
                _ = tokio::time::sleep(ttl) => {
                    self.emit(
                        run_id,
                        "tool_approval_resolved",
                        serde_json::json!({
                            "tool_call_id": tool_call_id,
                            "approval_id": record.approval_id,
                            "granted": false,
                        }),
                    )
                    .await;
                    ApprovalOutcome::Deny
                }
            };
            self.set_run_state(run_id, RunStatus::Running, serde_json::json!({}))
                .await;

            let granted = matches!(outcome, ApprovalOutcome::Approve { .. });
            if granted {
                self.emit(
                    run_id,
                    "tool_approval_resolved",
                    serde_json::json!({
                        "tool_call_id": tool_call_id,
                        "approval_id": record.approval_id,
                        "granted": true,
                    }),
                )
                .await;
            }
            if !granted {
                return self.deny_tool(run_id, record, "denied by user").await;
            }
        }

        self.emit(
            run_id,
            "tool_started",
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "name": tool_name,
                "arguments": arguments,
            }),
        )
        .await;

        let ctx = ToolContext {
            run_id,
            workspace_id: request.workspace_id,
            root_path: request.root_path.clone(),
            approval_token,
        };
        let outcome = {
            let registry = self.tools.read().await;
            registry.dispatch(ctx, &tool_name, arguments.clone()).await
        };

        match outcome {
            Ok(result) => {
                record.result = Some(result.content.clone());
                record.error = result.error.clone();
                let status = if result.ok { "succeeded" } else { "failed" };
                let err_text = result.error.clone();
                let _ = self
                    .executor
                    .record_tool_call(
                        run_id,
                        &tool_call_id,
                        &tool_name,
                        arguments,
                        status,
                        Some(result),
                        err_text.as_deref(),
                    )
                    .await;
            }
            Err(err) => {
                record.error = Some(err.to_string());
                let _ = self
                    .executor
                    .record_tool_call(
                        run_id,
                        &tool_call_id,
                        &tool_name,
                        arguments,
                        "failed",
                        None,
                        Some(&err.to_string()),
                    )
                    .await;
            }
        }

        self.emit(
            run_id,
            "tool_result",
            serde_json::json!({
                "tool_call_id": record.id,
                "name": record.name,
                "arguments": record.arguments,
                "result": record.result,
                "error": record.error,
            }),
        )
        .await;

        record
    }

    /// Look up the cancellation handle for a Run, if it is still active.
    async fn cancellation_of(&self, run_id: RunId) -> RunCancellation {
        // The handle is removed on completion, so fall back to a standalone
        // (never-notified) handle rather than blocking forever.
        match self.active.lock().await.get(&run_id) {
            Some(c) => c.clone(),
            None => RunCancellation::new(),
        }
    }

    /// Record a refused Tool call and tell the model why, so it can adapt
    /// instead of retrying blindly.
    async fn deny_tool(&self, run_id: RunId, record: RunToolCall, reason: &str) -> RunToolCall {
        let message = format!("Tool `{}` was not run: {}", record.name, reason);
        let _ = self
            .executor
            .record_tool_call(
                run_id,
                &record.id,
                &record.name.clone(),
                record.arguments.clone(),
                "denied",
                None,
                Some(&message),
            )
            .await;
        self.emit(
            run_id,
            "tool_denied",
            serde_json::json!({
                "tool_call_id": record.id,
                "name": record.name,
                "reason": reason,
            }),
        )
        .await;
        self.emit(
            run_id,
            "tool_result",
            serde_json::json!({
                "tool_call_id": record.id,
                "name": record.name,
                "arguments": record.arguments,
                "result": message,
                "error": message,
            }),
        )
        .await;
        RunToolCall {
            error: Some(message),
            ..record
        }
    }

    /// One Provider turn: stream and accumulate text / reasoning / Tool calls.
    async fn run_turn(
        &self,
        run_id: RunId,
        provider: &dyn Provider,
        request: &RunRequest,
        messages: &[Message],
        system: &str,
    ) -> anyhow::Result<TurnResult> {
        let req = ProviderRequest {
            model: request.model_id.clone(),
            messages: messages.to_vec(),
            system: if system.trim().is_empty() {
                None
            } else {
                Some(system.to_string())
            },
            tools: request.tools.clone(),
            temperature: request.temperature,
            top_p: None,
            max_tokens: request.max_tokens,
            reasoning: request.reasoning,
            stream: true,
        };

        self.emit(
            run_id,
            "model_started",
            serde_json::json!({ "model_id": request.model_id }),
        )
        .await;

        let stream = provider.stream(req).await?;
        let mut s = stream;
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut usage: Option<Usage> = None;
        let mut finish_reason: Option<String> = None;
        let mut error: Option<String> = None;
        let mut pending: HashMap<String, (String, String)> = HashMap::new();
        let mut order: Vec<String> = Vec::new();

        while let Some(ev) = futures_util::StreamExt::next(&mut s).await {
            match ev {
                StreamEvent::TextDelta(delta) => {
                    text.push_str(&delta);
                    self.emit(run_id, "text", serde_json::json!({ "delta": delta }))
                        .await;
                }
                StreamEvent::ReasoningDelta(delta) => {
                    reasoning.push_str(&delta);
                    self.emit(run_id, "reasoning", serde_json::json!({ "delta": delta }))
                        .await;
                }
                StreamEvent::ToolCallStart { id, name } => {
                    pending.entry(id.clone()).or_insert((name, String::new()));
                    order.push(id.clone());
                }
                StreamEvent::ToolCallArgumentDelta { id, delta } => {
                    if let Some(entry) = pending.get_mut(&id) {
                        entry.1.push_str(&delta);
                    }
                }
                StreamEvent::ToolCallFinish {
                    id,
                    name,
                    arguments,
                } => {
                    let entry = pending.entry(id.clone()).or_insert((name, String::new()));
                    if !arguments.is_empty() {
                        entry.1 = arguments;
                    }
                    if !order.contains(&id) {
                        order.push(id);
                    }
                }
                StreamEvent::Finished {
                    finish_reason: reason,
                    usage: u,
                } => {
                    finish_reason = Some(reason);
                    usage = u;
                    break;
                }
                StreamEvent::Error(err) => {
                    error = Some(err);
                    break;
                }
            }
        }

        if error.is_none() && finish_reason.is_none() {
            // The connection dropped or the server closed the stream early.
            // Without this the fragment collected so far was stored as a
            // complete answer, and the run's next turn looked like progress.
            tracing::warn!(
                %run_id,
                provider = provider.id(),
                text_chars = text.len(),
                "provider stream ended without a completion signal"
            );
            error = Some("the provider stream ended without a completion signal".to_string());
        }

        let tool_calls: Vec<AssistantToolCall> = order
            .iter()
            .filter_map(|id| {
                pending.get(id).map(|(name, args)| AssistantToolCall {
                    id: id.clone(),
                    kind: "function".to_string(),
                    function: AssistantToolCallFn {
                        name: name.clone(),
                        arguments: args.clone(),
                    },
                })
            })
            .collect();

        Ok(TurnResult {
            text,
            reasoning,
            tool_calls,
            usage,
            finish_reason,
            error,
        })
    }
}

/// The raw stored budget object, so the caller can decide what the mode's
/// budget is before applying the overrides in it.
async fn budget_snapshot(run_id: &RunId, engine: &RunEngine) -> Option<serde_json::Value> {
    let row: Option<(String,)> =
        sqlx::query_as::<_, (String,)>("SELECT data FROM runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_optional(engine.storage.pool())
            .await
            .unwrap_or(None);
    let data: serde_json::Value = row
        .and_then(|(raw,)| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    data.get("budget").cloned()
}

struct TurnResult {
    text: String,
    reasoning: String,
    tool_calls: Vec<AssistantToolCall>,
    usage: Option<Usage>,
    finish_reason: Option<String>,
    error: Option<String>,
}

/// Remove a leading `system` head block injected by the compression step.
fn strip_head(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .skip_while(|m| m.role == "system" && m.content.starts_with("<task_state>"))
        .collect()
}

/// Ask the Provider to compress older turns. Returns `None` when the Provider
/// cannot be reached so the caller keeps the original history.
async fn summarize_turns(
    provider: &dyn Provider,
    model: &str,
    older: &[Message],
) -> Option<String> {
    let mut transcript = String::new();
    for m in older {
        transcript.push_str(&format!("{}: {}\n", m.role, m.content));
    }
    let req = ProviderRequest {
        model: model.to_string(),
        messages: vec![Message::text(
            "user",
            format!(
                "以下の会話を、後続ターンで参照できる高密度な要約に圧縮してください。\
                 重要な事実・決定・未完了の作業・ファイルパスを必ず残してください。\n\n{}",
                transcript
            ),
        )],
        system: Some("You compress conversation history without losing actionable detail.".into()),
        tools: Vec::new(),
        temperature: Some(0.2),
        top_p: None,
        max_tokens: Some(1200),
        reasoning: None,
        stream: true,
    };
    let mut stream = provider.stream(req).await.ok()?;
    let mut out = String::new();
    while let Some(ev) = futures_util::StreamExt::next(&mut stream).await {
        match ev {
            StreamEvent::TextDelta(delta) => out.push_str(&delta),
            StreamEvent::Finished { .. } => break,
            StreamEvent::Error(_) => return None,
            _ => {}
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn history_to_message(h: &HistoryMessage) -> Message {
    Message {
        role: h.role.clone(),
        content: h.content.clone(),
        tool_calls: h.tool_calls.clone(),
        tool_call_id: h.tool_call_id.clone(),
        name: h.name.clone(),
        is_error: h.is_error,
    }
}

/// Render the block that is prepended to every Provider request: the goal the
/// user stated plus whatever checklist the agent has built with the `todo` tool.
///
/// Two separate concerns, one block, so the adapter can fold it into whichever
/// system channel it uses.
async fn render_head(
    task_state: &crate::context::TaskState,
    todos: &mixd_tools::todo::TodoStore,
) -> String {
    let mut head = task_state.render();
    let checklist = todos.render().await;
    if !checklist.is_empty() {
        head.push('\n');
        head.push_str(&checklist);
        head.push('\n');
    }
    head
}

fn message_to_history(m: &Message) -> HistoryMessage {
    HistoryMessage {
        role: m.role.clone(),
        content: m.content.clone(),
        tool_call_id: m.tool_call_id.clone(),
        name: m.name.clone(),
        tool_calls: m.tool_calls.clone(),
        is_error: m.is_error,
    }
}
