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
use crate::runs::verification::{
    destructive_reason, exit_code_of, mutated_paths, VerificationLedger,
};
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
    /// Wakes [`RunEngine::await_budget_extension`] the moment `resolve_budget`
    /// records an answer. The DB row remains the fallback (polled every
    /// [`BUDGET_DB_POLL_INTERVAL`]) for answers written before a restart.
    pub budget_notify: Arc<tokio::sync::Notify>,
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
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let executor = Arc::new(mixd_tools::execute::ToolExecutor::new(
            storage.clone(),
            secrets,
        ));
        // Keep enough snapshots for the longest run: agent allows 300 steps at
        // every-10 checkpointing (30 snapshots) plus headroom. The trim is per
        // run (`WHERE run_id = ?`), so a smaller keep would evict the early
        // checkpoints a long run would most want to resume from.
        let checkpoints = Arc::new(CheckpointStore::new(CHECKPOINT_KEEP));
        Self {
            storage,
            providers: Arc::new(ProviderRegistry::default()),
            tools: Arc::new(tokio::sync::RwLock::new(tools)),
            executor,
            checkpoints,
            policy: AgentModePolicy::default(),
            events: Arc::new(tx),
            sequences: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            event_buffer: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            flush_scheduled: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            active: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            budget_decisions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            budget_notify: Arc::new(tokio::sync::Notify::new()),
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
        // Cloning the event is the only cost of publishing it, and it is pure
        // waste when nothing is listening — scheduled and resumed runs have no
        // subscriber at all. A receiver attached later cannot see a message
        // published before it subscribed, so skipping the send is not a
        // behavioural change.
        if self.events.receiver_count() > 0 {
            if let Err(err) = self.events.send(evt.clone()) {
                // Only reachable on a receiver-count race; a dropped event
                // here is still durable via the replay log below.
                tracing::debug!(%run_id, kind, sequence, %err, "broadcast send had no live receiver");
            }
        }
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
        self.persist_batch(run_id, &batch).await;
    }

    /// Drain every Run's buffer. Used before a replay so reloading a
    /// conversation can never show a transcript the log has not caught up on.
    pub async fn flush_all_events(&self) {
        let all = {
            let mut buffer = self.event_buffer.lock().await;
            std::mem::take(&mut *buffer)
        };
        for (run_id, batch) in all {
            self.persist_batch(run_id, &batch).await;
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
        // Double-checked: another task may have seeded while the query ran.
        // If so, just increment the existing counter so every caller gets a
        // unique value under the lock; never overwrite a newer counter with a
        // stale base.
        if let Some(entry) = seqs.get_mut(&run_id) {
            *entry += 1;
            return *entry;
        }
        let next = base.saturating_add(1);
        seqs.insert(run_id, next);
        next
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

    /// Persist a drained buffer with one statement rather than one round trip
    /// per event.
    ///
    /// Token deltas arrive in bursts of up to `EVENT_FLUSH_BATCH`, so writing
    /// them one at a time made SQLite the pacing item of a streaming turn. A
    /// multi-row `INSERT` is atomic: when it is rejected it has written
    /// nothing, so the per-row path — which renumbers on a sequence collision —
    /// can be retried without risking duplicates.
    async fn persist_batch(&self, run_id: RunId, batch: &[RunEvent]) {
        if batch.is_empty() {
            return;
        }
        if let Err(err) = self.insert_events(batch).await {
            tracing::warn!(
                %run_id,
                %err,
                "batched run event insert failed; falling back to row by row"
            );
            for evt in batch {
                if let Err(err) = self.persist_event(evt).await {
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

    async fn insert_events(&self, batch: &[RunEvent]) -> anyhow::Result<()> {
        for chunk in batch.chunks(EVENT_FLUSH_BATCH) {
            let mut sql =
                String::from("INSERT INTO run_events (id, run_id, sequence, kind, data) VALUES ");
            let mut separator = "";
            for _ in chunk {
                sql.push_str(separator);
                sql.push_str("(?, ?, ?, ?, ?)");
                separator = ", ";
            }
            let mut statement = sqlx::query(&sql);
            for evt in chunk {
                statement = statement
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(evt.run_id.to_string())
                    .bind(evt.sequence)
                    .bind(&evt.kind)
                    .bind(evt.data.to_string());
            }
            statement.execute(self.storage.pool()).await?;
        }
        Ok(())
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
        let registry = self.tools.read().await;
        super::tool_specs::map_tool_specs(&registry.specs())
    }

    /// Mode-filtered specs (Docker `allowed_modes` parity). Chat excludes
    /// delegation and planning tools; thinking excludes delegation only.
    ///
    /// Takes the [`Mode`] enum rather than a string: unknown mode strings are
    /// rejected at the caller's parse boundary (with a warning + chat fallback)
    /// instead of silently degrading inside the engine.
    pub async fn tool_specs_for_mode(&self, mode: Mode) -> Vec<mixd_providers::ToolSpec> {
        let registry = self.tools.read().await;
        super::tool_specs::map_tool_specs(&registry.specs_for_mode(mode.as_str()))
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
            .await;
        match row {
            Ok(Some(row)) => run_view_from_row(&row),
            Ok(None) => None,
            Err(err) => {
                tracing::error!(?err, %id, "run_state query failed");
                None
            }
        }
    }

    /// All Runs of a conversation, oldest first — the persisted counterpart of
    /// the live event stream, used to rebuild tool activity for past turns.
    ///
    /// A query failure is logged with the conversation id and yields an empty
    /// list (the UI shows "no runs" rather than failing the whole view); a
    /// corrupt row is skipped with its own log in [`run_view_from_row`], and
    /// the skip count is warned here so silent drops stay visible.
    pub async fn list_runs(&self, conversation_id: uuid::Uuid) -> Vec<RunView> {
        let rows = sqlx::query(
            "SELECT id, conversation_id, data FROM runs \
             WHERE conversation_id = ? ORDER BY created_at ASC",
        )
        .bind(conversation_id.to_string())
        .fetch_all(self.storage.pool())
        .await
        .unwrap_or_else(|err| {
            tracing::error!(?err, %conversation_id, "list_runs query failed");
            Vec::new()
        });
        let mut skipped = 0usize;
        let views: Vec<RunView> = rows
            .iter()
            .filter_map(|row| {
                run_view_from_row(row).or_else(|| {
                    skipped += 1;
                    None
                })
            })
            .collect();
        if skipped > 0 {
            tracing::warn!(%conversation_id, skipped, "list_runs skipped corrupt run rows");
        }
        views
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
            // SQLite JSON path syntax: keys with special chars are quoted as
            // $."key", and a literal double-quote inside the key is escaped
            // as "" (not \"). Backslashes need no escaping in paths.
            let safe_key = k.replace('"', "\"\"");
            binds.push(format!("$.\"{safe_key}\""));
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
        // Clone under the guard, drop it, then await the catalogue locks so a
        // concurrent `replace_tools` is never queued behind them.
        let registry = self.tools.read().await.clone();
        registry.invalidate_catalogues().await;
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

    /// Queue post-response memory formation (memory spec §6 自動形成).
    ///
    /// Called once per finished Run, right after the final assistant turn is
    /// persisted — 「ユーザーの発言とアシスタントの応答が 1 セットになったあとに
    /// 1 回」 — and always on a background task so the response path never
    /// waits on the queue. A temporary turn deliberately skips it: an
    /// exchange that leaves no trace must not leave a memory either.
    fn spawn_memory_extraction(&self, run_id: RunId, user_message: &str, assistant: &str) {
        let storage = self.storage.clone();
        let run_id = run_id.to_string();
        let user = user_message.to_string();
        let assistant = assistant.to_string();
        tokio::spawn(async move {
            match crate::memory_jobs::enqueue(&storage, &run_id, &user, &assistant).await {
                Ok(true) => tracing::debug!(%run_id, "memory extraction queued"),
                // No `runs` row (a sub-agent turn) or already queued: both
                // mean "no job for this id", which is the contract.
                Ok(false) => {}
                Err(err) => tracing::warn!(%run_id, %err, "queueing memory extraction failed"),
            }
        });
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
///
/// Corrupt JSON is an error (and the row is skipped); individual missing or
/// unparseable fields fall back to defaults with a warning so one bad field
/// does not hide the whole run, but the corruption stays visible in logs.
fn run_view_from_row(row: &sqlx::sqlite::SqliteRow) -> Option<RunView> {
    use sqlx::Row;
    let id = uuid::Uuid::parse_str(&row.try_get::<String, _>("id").ok()?).ok()?;
    let conversation_id = row
        .try_get::<String, _>("conversation_id")
        .ok()
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())?;
    let data: serde_json::Value = match serde_json::from_str(
        &row.try_get::<String, _>("data").ok()?,
    ) {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(?err, run_id = %id, "run_view_from_row: corrupt JSON blob, skipping row");
            return None;
        }
    };
    let status_raw = data.get("status").and_then(|v| v.as_str());
    let status = status_raw.and_then(parse_status).unwrap_or_else(|| {
        if status_raw.is_some() {
            tracing::warn!(run_id = %id, status = status_raw, "unknown run status; showing queued");
        }
        RunStatus::Queued
    });
    let mode = data.get("mode").and_then(|v| v.as_str());
    if mode.is_none() {
        tracing::warn!(run_id = %id, "run row has no mode; showing chat");
    }
    if data.get("modelId").and_then(|v| v.as_str()).is_none() {
        tracing::debug!(run_id = %id, "run row has no modelId; showing empty");
    }
    Some(RunView {
        id,
        conversation_id,
        mode: mode.unwrap_or("chat").to_string(),
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
        error: data
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
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

/// Live-event broadcast buffer: enough for a bursty streamed turn without
/// prospectively allocating for a whole run. Receivers that cannot keep up see
/// `Lagged`, never a block of the engine.
const BROADCAST_CAPACITY: usize = 4096;

/// Checkpoint snapshots kept per run: agent allows 300 steps at every-10
/// checkpointing (30 snapshots) plus headroom. The trim is per run
/// (`WHERE run_id = ?`), so a smaller keep would evict the early checkpoints
/// a long run would most want to resume from.
const CHECKPOINT_KEEP: usize = 32;

/// Characters of the user's message kept as the run's goal head block.
const GOAL_PREVIEW_CHARS: usize = 200;

/// Summariser turn: low temperature for fidelity, capped well below a full
/// turn so compression never eats the budget it is trying to save.
const SUMMARIZER_TEMPERATURE: f32 = 0.2;
const SUMMARIZER_MAX_TOKENS: u32 = 1200;

/// Budget-extension answer delivery: `resolve_budget` wakes the waiter via
/// `budget_notify` immediately; the DB row is only a fallback for answers
/// written before a restart, so it is polled on this slow cadence instead of
/// the old 250 ms busy loop.
const BUDGET_DB_POLL_INTERVAL: Duration = Duration::from_secs(5);

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
    /// Extra read-only folders the tools may read from (Codex `--add-dir`
    /// parity for monorepos).
    pub additional_paths: Vec<String>,
    pub mode: Mode,
    pub model_id: String,
    pub provider_id: String,
    pub system: Option<String>,
    /// The project's own check (`pnpm rust:test`, `cargo test`, …).
    ///
    /// When set, a Run that changed files may not conclude until that command
    /// has been run *successfully* — the second half of "did you verify this?",
    /// which reading the file back cannot answer. `None` disables the gate.
    pub verify_command: Option<String>,
    /// A formatter or lint command the project wants applied around edits.
    ///
    /// Reported to the model as part of the verification hint instead of being
    /// executed behind its back: a hidden process rewriting files mid-run makes
    /// the transcript disagree with the disk, and it would need its own
    /// approval semantics on top of the tool gate.
    pub format_command: Option<String>,
    /// The selected model's context window, or `0` when unknown.
    ///
    /// Drives context compression, which used to be a fixed character count and
    /// therefore both late for a small local model and wasteful for a large one.
    pub context_window: u32,
    pub temperature: Option<f32>,
    pub reasoning: Option<ReasoningControl>,
    pub max_tokens: Option<u32>,
    /// A throwaway turn: Memory / Knowledge are not injected, and the app layer
    /// purges the transcript afterwards (Docker edition's "temporary mode").
    ///
    /// The Engine only honours the half it owns — suppressing retrieval — so a
    /// caller that never purges still gets a run with no memory influence.
    pub temporary: bool,
    /// Shared across the run's turns, so `run_turn` never deep-copies the spec.
    pub tools: Arc<Vec<ToolSpec>>,
    pub history: Vec<HistoryMessage>,
    pub user_message: String,
    pub resume_from: Option<CheckpointSnapshot>,
}

impl RunRequest {
    /// The model name as the Provider API expects it. See
    /// [`wire_model_for`].
    pub fn wire_model(&self) -> &str {
        wire_model_for(&self.model_id, &self.provider_id)
    }
}

/// Strip a stored `<provider_id>:<model>` id down to the bare `<model>`.
///
/// `model_id` is stored — and handed to the Engine — as `<provider_id>:<model>`,
/// because the same short model name (`gpt-4o`) can exist under two different
/// providers. That prefix is an internal identity, not part of the model name:
/// OpenAI, Anthropic and Gemini all forward the string verbatim, so sending it
/// unchanged makes every request fail with "model not found".
///
/// Only an *exact* `<provider_id>:` prefix is removed, and only when what
/// follows it starts at a `:`. A bare id (`gpt-4o`), an id whose provider no
/// longer matches, and a local-runtime tag that contains its own colon
/// (`llama3.1:8b`) all pass through untouched.
pub fn wire_model_for<'a>(model_id: &'a str, provider_id: &str) -> &'a str {
    if provider_id.is_empty() {
        return model_id;
    }
    match model_id.strip_prefix(provider_id) {
        Some(rest) => rest.strip_prefix(':').unwrap_or(model_id),
        None => model_id,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointSnapshot {
    pub step_index: u32,
    /// Tool calls completed before this checkpoint, so a resumed Run continues
    /// its budget instead of starting the allowance over.
    #[serde(default)]
    pub tool_calls: u32,
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
    /// Drop the per-run handles once a Run reaches a terminal state.
    ///
    /// `active` and `sequences` are keyed by `RunId` and used to be cleared on
    /// the happy path only, so a cancelled, declined or failed Run left an entry
    /// behind for the lifetime of the process.
    async fn release_run(&self, run_id: RunId) {
        self.active.lock().await.remove(&run_id);
        self.sequences.lock().await.remove(&run_id);
        self.flush_scheduled.lock().await.remove(&run_id);
    }

    /// Record a Run's terminal state and return its outcome.
    ///
    /// Every exit path of the agent loop ends here. The four things a finished
    /// Run has to do — persist the status, emit it (which also drains the
    /// token buffer) and drop the per-run handles — used to be spelled out five
    /// separate times, and a path that skipped `release_run` left its `active`
    /// entry behind for the lifetime of the process.
    async fn finish_run(
        &self,
        run_id: RunId,
        status: RunStatus,
        state: serde_json::Value,
        payload: serde_json::Value,
        outcome: RunOutcome,
    ) -> anyhow::Result<RunOutcome> {
        self.set_run_state(run_id, status, state).await;
        self.emit(run_id, "status", payload).await;
        self.release_run(run_id).await;
        Ok(RunOutcome { status, ..outcome })
    }

    /// Shared terminal path for the three cancellation exits (pre-turn check,
    /// budget-wait cancel, mid-stream cancel). They used to spell the same
    /// `finish_run(Cancelled)` inline three times; a field added to one copy
    /// (e.g. the `reasoning` added to the completed path) was silently missing
    /// from the others.
    #[allow(clippy::too_many_arguments)]
    async fn finish_cancelled(
        &self,
        run_id: RunId,
        text: String,
        reasoning: String,
        tool_calls: Vec<RunToolCall>,
        usage: Option<Usage>,
        finish_reason: Option<String>,
        steps: u32,
    ) -> anyhow::Result<RunOutcome> {
        self.finish_run(
            run_id,
            RunStatus::Cancelled,
            serde_json::json!({}),
            serde_json::json!({ "status": "cancelled" }),
            RunOutcome {
                status: RunStatus::Cancelled,
                text,
                reasoning,
                tool_calls,
                usage,
                finish_reason,
                steps,
            },
        )
        .await
    }

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
            // A temporary turn skips retrieval entirely: the point of the mode
            // is that this exchange leaves no trace, and a memory written from
            // it would do exactly the opposite.
            if !request.temporary {
                match crate::retrieval::retrieve_all(self.storage.as_ref(), &request.user_message)
                    .await
                {
                    Ok(block) => {
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
                    Err(err) => {
                        tracing::warn!(%run_id, %err, "memory/knowledge retrieval failed; continuing without injected context");
                    }
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

        let mut steps: u32 = request
            .resume_from
            .as_ref()
            .map(|s| s.step_index)
            .unwrap_or(0);
        let mut total_tool_calls: u32 = request
            .resume_from
            .as_ref()
            .map(|s| s.tool_calls)
            .unwrap_or(0);
        let mut all_tool_calls: Vec<RunToolCall> = Vec::new();
        let mut final_text = String::new();
        let mut final_reasoning = String::new();
        let mut last_usage: Option<Usage> = None;
        let mut last_finish: Option<String> = None;
        let mut stagnation = StagnationDetector::new();
        let mut verification = VerificationLedger::new(request.verify_command.clone());
        // Signals for the current turn, collected as tool calls execute. Kept
        // out here because the classifier needs the tool result, which only
        // exists after dispatch.
        let mut signals: Vec<crate::runs::stagnation::TurnSignal> = Vec::new();
        // How many extra turns the agent has been given to verify an
        // unconfirmed destructive call. Bounded so a model that never reads
        // anything back cannot loop forever.
        let mut unverified_repair_turns: u32 = 0;
        // Whether the engine has already run the project's check automatically.
        // Without this a failing check would re-arm the gate every turn and
        // loop until the step budget is exhausted.
        let mut auto_verified = false;
        // Last checklist emitted, so the UI only gets an event when it changes.
        let mut last_todos: Vec<mixd_tools::todo::TodoItem> = Vec::new();
        let mut summary = crate::context::TieredSummary::new();
        let task_state = crate::context::TaskState {
            goal: request
                .user_message
                .chars()
                .take(GOAL_PREVIEW_CHARS)
                .collect(),
            ..Default::default()
        };
        // The skill catalogue belongs in the system prompt, not in the history:
        // it is the same on every turn and the adapters cache it there.
        //
        // The built-in procedure is composed in here rather than by each caller
        // so every entry point — chat, agent, resumed run, scheduled run,
        // sub-agent — gets it from one place, and a caller that passes no system
        // text no longer sends an empty prompt.
        let mut system = crate::prompt::compose_system_for_run(
            request.system.as_deref(),
            request.mode,
            !request.tools.is_empty(),
        )
        .unwrap_or_default();
        // Snapshot the registry before awaiting its catalogue: holding the read
        // guard across `skills_prompt(...).await` blocked an MCP reconnect's
        // write lock for the whole lookup.
        let registry = self.tools.read().await.clone();
        let skills_prompt = registry.skills_prompt(request.root_path.as_deref()).await;
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
                return self
                    .finish_cancelled(
                        run_id,
                        final_text,
                        final_reasoning,
                        all_tool_calls,
                        last_usage,
                        last_finish,
                        steps,
                    )
                    .await;
            }

            // Budget gate before each Provider turn.
            match budget.check(steps, total_tool_calls).await {
                BudgetVerdict::Ok => {}
                BudgetVerdict::Exhausted(reason) => {
                    let Some(grant) = self.await_budget_extension(run_id, reason, &budget).await
                    else {
                        // Cancelled while waiting: reporting `completed` here
                        // overwrote the user's cancellation.
                        return self
                            .finish_cancelled(
                                run_id,
                                final_text,
                                final_reasoning,
                                all_tool_calls,
                                last_usage,
                                last_finish,
                                steps,
                            )
                            .await;
                    };
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
                        return self
                            .finish_run(
                                run_id,
                                RunStatus::Completed,
                                serde_json::json!({}),
                                serde_json::json!({ "status": "completed", "reason": stop_reason }),
                                RunOutcome {
                                    status: RunStatus::Completed,
                                    text: final_text,
                                    reasoning: final_reasoning,
                                    tool_calls: all_tool_calls,
                                    usage: last_usage,
                                    finish_reason: last_finish,
                                    steps,
                                },
                            )
                            .await;
                    }
                }
            }

            steps += 1;

            // Hierarchical context compression. The head block — the task goal
            // plus the agent's own checklist — is prepended so both survive the
            // folding of older turns. Without the checklist an agent loses track
            // of a five-step refactor as soon as the context compresses.
            //
            // The head and the checklist are read under one shared guard, and
            // only once per step: rendering the block twice meant formatting
            // the whole checklist two times and taking the registry lock three
            // times on every pass through the loop. Deriving both from one
            // snapshot is what keeps the plan the model is acting on identical
            // to the plan the UI shows.
            let (head, checklist) = {
                // Clone the checklist handle while the registry guard is held,
                // then drop it: `render_head` and `snapshot` await the store's
                // own lock, and holding the registry read guard across them
                // stalled an MCP reconnect's write lock for the whole render.
                let (todos, tiered) = {
                    let tools = self.tools.read().await;
                    (tools.todos.clone(), self.policy.context_tiered_summary)
                };
                let head = if tiered {
                    render_head(&task_state, &todos, run_id).await
                } else {
                    String::new()
                };
                (head, todos.snapshot(run_id).await)
            };
            // Surface the checklist to the UI, not just to the model: a plan the
            // user cannot see is a plan they cannot correct.
            if checklist != last_todos {
                let payload = serde_json::json!({
                    "todos": checklist
                        .iter()
                        .map(|item| serde_json::json!({
                            "id": item.id,
                            "content": item.content,
                            "status": item.status,
                        }))
                        .collect::<Vec<_>>(),
                });
                last_todos = checklist;
                self.emit(run_id, "todo_list", payload).await;
            }

            // One history list for the rest of the step: the compression pass
            // and the request below both work on it, and the Provider takes it
            // over afterwards. This used to be two separate clones of the whole
            // conversation per step, plus a third inside `compress`.
            let mut current = if head.trim().is_empty() {
                messages.clone()
            } else {
                let mut v = Vec::with_capacity(messages.len() + 1);
                v.push(Message::text("system", head));
                v.extend(messages.iter().cloned());
                v
            };

            if self.policy.context_tiered_summary {
                let source_messages = current.len();
                // The summariser opens its own Provider turn, so it needs the
                // bare model name for the same reason `run_turn` does.
                let model = request.wire_model().to_string();
                let provider_handle = provider.clone();
                // The summariser is handed exactly the slice being dropped, so
                // the summary never duplicates the tail kept verbatim. The
                // history and the summary are moved in and handed straight
                // back, so an ordinary (uncompressed) step allocates nothing.
                let outcome = crate::context::compress_with_budget(
                    std::mem::take(&mut current),
                    std::mem::take(&mut summary),
                    request.context_window,
                    |older| async move {
                        summarize_turns(provider_handle.as_ref(), &model, &older).await
                    },
                )
                .await;
                let evicted = outcome.evicted_tool_results;
                current = outcome.messages;
                summary = outcome.summary;
                if !outcome.skipped || evicted > 0 {
                    messages = strip_head(&current);
                    self.emit(
                        run_id,
                        "context_summary",
                        serde_json::json!({
                            "compressed": !outcome.skipped,
                            "activeChars": summary.char_count(),
                            "archiveRows": summary.archive.len(),
                            "messages": messages.len(),
                            "sourceMessages": source_messages,
                            "evictedToolResults": evicted,
                        }),
                    )
                    .await;
                }
            }

            let turn = self
                .run_turn(run_id, provider.as_ref(), &request, current, &system)
                .await?;

            // A cancellation observed mid-stream comes back as `turn.error`;
            // report it as Cancelled rather than as a failure.
            if cancellation.is_cancelled() {
                return self
                    .finish_cancelled(
                        run_id,
                        final_text,
                        final_reasoning,
                        all_tool_calls,
                        last_usage,
                        last_finish,
                        steps,
                    )
                    .await;
            }

            if let Some(err) = turn.error {
                self.emit(run_id, "error", serde_json::json!({ "error": err }))
                    .await;
                return self
                    .finish_run(
                        run_id,
                        RunStatus::Failed,
                        serde_json::json!({ "error": err }),
                        serde_json::json!({ "status": "failed", "error": err }),
                        RunOutcome {
                            status: RunStatus::Failed,
                            text: final_text,
                            reasoning: final_reasoning,
                            tool_calls: all_tool_calls,
                            usage: last_usage,
                            finish_reason: last_finish,
                            steps,
                        },
                    )
                    .await;
            }

            // Spend the turn against the token ceiling. `last_usage` is the
            // *latest* turn's usage, which the UI shows per turn; the budget
            // needs the running total.
            if let Some(usage) = &turn.usage {
                budget.record_usage(usage.input_tokens, usage.output_tokens);
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
            // `take`, not `clone`: the detector sees each signal exactly once.
            // Cloning let every turn's signals be re-fed on every later turn, so
            // the window filled with duplicates after three real failures and
            // kept firing the repair hint forever.
            let turn_signals = std::mem::take(&mut signals);
            if self.policy.stagnation_detection {
                if let Some(report) = stagnation.observe(turn_signals) {
                    self.emit(
                        run_id,
                        "stagnation_detected",
                        serde_json::json!({ "hint": report.hint, "kind": report.kind }),
                    )
                    .await;

                    // Phase 8.2: Turn-level re-retrieval on stagnation
                    if !request.temporary {
                        let query = format!("{} {}", request.user_message, report.hint);
                        if let Ok(block) =
                            crate::retrieval::retrieve_all(self.storage.as_ref(), &query).await
                        {
                            if let Some(msg) = block.to_message() {
                                messages.push(msg);
                            }
                        }
                    }

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
                // Two independent questions, both gated here: "did you read the
                // effect back?" (`is_blocked`) and "did you run the project's
                // own check?" (`needs_verification`). Reading a file back
                // correctly proves nothing about whether it still builds.
                let unanswered = verification.is_blocked();
                let unchecked = verification.needs_verification();
                if self.policy.strict_verification && (unanswered || unchecked) {
                    if unverified_repair_turns >= MAX_UNVERIFIED_REPAIR_TURNS {
                        self.emit(
                            run_id,
                            "verification_failed",
                            serde_json::json!({
                                "outstanding": verification.outstanding(),
                                "unverified": verification.unverified(),
                                "reason": verification.last_reason(),
                            }),
                        )
                        .await;
                        final_text = turn.text.clone();
                        let final_message = Message::text("assistant", turn.text.clone());
                        // Persist it: `messages` is in-memory only, and the next
                        // turn of the conversation is rebuilt from this table.
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
                                    "model_id": request.model_id,
                                }),
                            )
                            .await;
                        }
                        if !request.temporary {
                            self.spawn_memory_extraction(
                                run_id,
                                &request.user_message,
                                &final_text,
                            );
                        }
                        break;
                    }
                    unverified_repair_turns += 1;
                    let outstanding = verification.outstanding();
                    let unverified = verification.unverified();
                    self.emit(
                        run_id,
                        "verification_required",
                        serde_json::json!({
                            "tool": outstanding.first().cloned().unwrap_or_default(),
                            "outstanding": outstanding,
                            "unverified": unverified,
                            "verifyCommand": verification.verify_command(),
                            "reason": verification.last_reason(),
                        }),
                    )
                    .await;
                    let mut hints: Vec<String> = Vec::new();
                    if unanswered {
                        hints.push(format!(
                            "You have not confirmed the effect of {}. Read the affected path \
                             back and state what is actually there before you conclude. \
                             Outstanding: {}.",
                            outstanding.join(", "),
                            verification
                                .last_reason()
                                .unwrap_or_else(|| "unconfirmed change".to_string())
                        ));
                    }
                    if unchecked {
                        let command = verification.verify_command().unwrap_or_default();
                        let format_note = request
                            .format_command
                            .as_deref()
                            .map(|cmd| format!(" Run `{cmd}` first if you have not."))
                            .unwrap_or_default();
                        hints.push(format!(
                            "You changed {} file(s) but have not run the project check. Run \
                             `{}` and report what it actually said before you conclude.{}\n\
                             Unverified: {}.",
                            unverified.len(),
                            command,
                            format_note,
                            unverified.join(", ")
                        ));
                    }
                    // Phase 8.2: Turn-level re-retrieval on verification checkpoint
                    if !request.temporary {
                        let query = format!("{} verify fix repair", request.user_message);
                        if let Ok(block) =
                            crate::retrieval::retrieve_all(self.storage.as_ref(), &query).await
                        {
                            if let Some(msg) = block.to_message() {
                                messages.push(msg);
                            }
                        }
                    }

                    messages.push(Message::text(
                        "user",
                        format!("[verification] {}", hints.join(" ")),
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
                if !request.temporary {
                    self.spawn_memory_extraction(run_id, &request.user_message, &final_text);
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

            // The argument blob is decoded once per call. Dispatch, the
            // stagnation detector and the verification ledger all read the very
            // same object, and each used to decode it again from the wire
            // string — six JSON parses per call, on every call of every step.
            let call_args: Vec<serde_json::Value> = turn
                .tool_calls
                .iter()
                .map(|c| {
                    serde_json::from_str(&c.function.arguments).unwrap_or_else(|err| {
                        tracing::warn!(tool = %c.function.name, %err, "tool arguments are not valid JSON; passing Null");
                        serde_json::Value::Null
                    })
                })
                .collect();

            let cancellation = self.cancellation_of(run_id).await;
            let mut tool_messages: Vec<Message> = Vec::new();
            for (call, args) in turn.tool_calls.iter().zip(call_args.iter()) {
                if cancellation.is_cancelled() {
                    break;
                }
                total_tool_calls += 1;
                if crate::runs::stagnation::is_read_only(&call.function.name, args)
                    && stagnation
                        .note_read(crate::runs::stagnation::read_key(&call.function.name, args))
                {
                    signals.push(crate::runs::stagnation::TurnSignal::RepeatedRead);
                }
                // Register the destructive effect *before* dispatch so a read
                // that follows it in the same turn can clear it. Doing this
                // after the loop meant a `write` then `read` in one assistant
                // turn left the gate open even though the read had confirmed it.
                let destructive = destructive_reason(&call.function.name, args);
                if self.policy.strict_verification && destructive.is_some() {
                    verification.requires_confirmation(&call.function.name, args);
                    self.emit(
                        run_id,
                        "destructive_tool_called",
                        serde_json::json!({
                            "tool": call.function.name,
                            "arguments": args,
                            "reason": destructive.clone().unwrap_or_default(),
                        }),
                    )
                    .await;
                }
                // A `terminal` call or a slow fetch must not make the Run
                // uncancellable: race the dispatch against the cancellation
                // signal and stop as soon as the user asks to.
                // Plan mode refuses anything that changes the workspace, before
                // the permission gate and therefore before any stored allow
                // rule can turn it writable. A refusal is reported through the
                // same denied-call path as a user denial, so the model gets a
                // reason it can act on instead of a silent no-op.
                let result = if request.mode.is_read_only()
                    && crate::runs::stagnation::is_mutating(&call.function.name, args)
                {
                    let record = RunToolCall {
                        id: call.id.clone(),
                        name: call.function.name.clone(),
                        arguments: args.clone(),
                        result: None,
                        error: None,
                        approval_id: None,
                    };
                    self.deny_tool(
                        run_id,
                        record,
                        "plan mode is read-only; describe the change and present it \
                         with `user_plan` instead of making it",
                    )
                    .await
                } else {
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled_signal() => {
                            tracing::info!(%run_id, tool = %call.function.name, "cancelled mid-tool; stopping");
                            break;
                        }
                        result = self.dispatch_tool(run_id, &request, call, args) => result,
                    }
                };
                all_tool_calls.push(result.clone());
                let is_error = result.error.is_some();
                // Only a read that actually succeeded closes the gate, and only
                // for the target it confirms.
                if !is_error && destructive.is_none() {
                    verification.confirm(&call.function.name, args);
                }
                if !is_error {
                    // A deliberate file change re-arms the check gate, and a
                    // successful `terminal` run of the configured check is the
                    // only thing that clears it.
                    for path in mutated_paths(&call.function.name, args) {
                        verification.note_mutation(&path);
                    }
                    if call.function.name == "terminal" {
                        let exit_ok = result
                            .result
                            .as_ref()
                            .and_then(exit_code_of)
                            .is_some_and(|code| code == 0);
                        verification.note_command(
                            args.get("command")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default(),
                            exit_ok,
                        );
                    }
                }
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
                signals.push(classify_tool_call(&call.function.name, args, is_error));
                tool_messages.push(Message::tool_result(call, content, is_error));
            }
            // If we stopped early on cancellation, the assistant turn already
            // declared every tool call; give the remaining ones a terminal
            // result so the persisted transcript stays valid for the next turn
            // (a dangling `tool_calls` turn is rejected by every provider).
            if cancellation.is_cancelled() && tool_messages.len() < turn.tool_calls.len() {
                for call in turn.tool_calls.iter().skip(tool_messages.len()) {
                    tool_messages.push(Message::tool_result(call, "Error: run cancelled", true));
                }
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

            // Auto-verify: when the project's check is configured and the
            // agent has changed files without running it, run the check
            // automatically. The model-driven hint is still emitted (so the
            // model can run it itself if it prefers), but the engine no longer
            // depends on the model noticing the hint. A successful run clears
            // the gate via `note_command`; a failing run leaves it open so the
            // model sees the output and can react.
            //
            // The guard on `auto_verified` prevents an infinite loop: if the
            // check itself fails, the gate stays open and the repair-turn
            // counter (bounded by `MAX_UNVERIFIED_REPAIR_TURNS`) eventually
            // stops the run.
            if self.policy.strict_verification
                && verification.needs_verification()
                && !auto_verified
            {
                if let Some(command) = verification.verify_command() {
                    let command = command.to_string();
                    auto_verified = true;
                    let ctx = ToolContext {
                        run_id,
                        workspace_id: request.workspace_id,
                        root_path: request.root_path.clone(),
                        additional_paths: request.additional_paths.clone(),
                        approval_token: None,
                    };
                    let result = registry
                        .dispatch(ctx, "terminal", serde_json::json!({ "command": command }))
                        .await;
                    let (ok, output) = match result {
                        Ok(r) => {
                            let exit_ok = r
                                .content
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .is_some_and(|c| c == 0);
                            (r.ok && exit_ok, r.content.to_string())
                        }
                        Err(e) => (false, format!("Error: {e}")),
                    };
                    verification.note_command(&command, ok);
                    let summary = if ok {
                        format!("[auto-verify] `{command}` passed.")
                    } else {
                        format!("[auto-verify] `{command}` failed:\n{output}")
                    };
                    messages.push(Message::text("user", summary));
                }
            }

            // A run that has actually verified its effects earns its repair
            // allowance back, so strict verification cannot permanently cap a
            // long multi-file run after two unconfirmed turns.
            if !verification.is_blocked() && !verification.needs_verification() {
                unverified_repair_turns = 0;
            }

            // Intermediate checkpointing. The interval is a public policy field
            // a caller can set, so a zero must not turn the modulo into a panic.
            if self.policy.checkpointing
                && self.policy.checkpoint_every_steps > 0
                && steps % self.policy.checkpoint_every_steps == 0
            {
                let snapshot = CheckpointSnapshot {
                    step_index: steps,
                    tool_calls: total_tool_calls,
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

        self.finish_run(
            run_id,
            RunStatus::Completed,
            serde_json::json!({
                "stepCount": steps,
                "toolCallCount": total_tool_calls,
            }),
            serde_json::json!({
                "status": "completed",
                "steps": steps,
                "tool_calls": total_tool_calls,
                "usage": last_usage,
                "tokensUsed": budget.tokens_used,
                "tokenLimit": budget.token_limit,
                "estimatedCostUsd": last_usage
                    .as_ref()
                    .and_then(|usage| crate::pricing::estimate_cost_usd(request.wire_model(), usage)),
            }),
            RunOutcome {
                status: RunStatus::Completed,
                text: final_text,
                reasoning: final_reasoning,
                tool_calls: all_tool_calls,
                usage: last_usage,
                finish_reason: last_finish,
                steps,
            },
        )
        .await
    }

    /// Ask the user for a budget extension. Returns `None` when the Run was
    /// cancelled while waiting, `Some(granted)` otherwise.
    async fn await_budget_extension(
        &self,
        run_id: RunId,
        reason: &str,
        budget: &Budget,
    ) -> Option<bool> {
        // An answer is persisted in `runs.data.budgetDecision` and nothing ever
        // cleared it, so the *second* prompt of a Run — or a resumed one — was
        // answered instantly with the first prompt's verdict instead of asking.
        self.budget_decisions.lock().await.remove(&run_id);
        // If this write fails the stale decision stays on the row and the next
        // prompt is auto-answered with the previous verdict, so the failure has
        // to be visible rather than discarded.
        if let Err(err) = sqlx::query(
            "UPDATE runs SET data = json_remove(data, '$.budgetDecision') \
             WHERE id = ? AND data IS NOT NULL",
        )
        .bind(run_id.to_string())
        .execute(self.storage.pool())
        .await
        {
            tracing::error!(
                %run_id,
                %err,
                "clearing the persisted budget decision failed; a later prompt may be auto-answered"
            );
        }
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
                "tokenLimit": budget.token_limit,
                "tokensUsed": budget.tokens_used,
                "grants": budget.grants_used + 1,
            }),
        )
        .await;
        // Wait for the UI to answer via `budget_extension`: woken immediately
        // by `resolve_budget`'s notify, with the DB row as a fallback for
        // answers persisted before a restart (polled on a slow cadence, not
        // the old 250 ms busy loop).
        let deadline =
            std::time::Instant::now() + Duration::from_secs(budget.extension_timeout_secs);
        let cancellation = self.cancellation_of(run_id).await;
        let granted = loop {
            // A cancelled Run must not sit here until the deadline and then be
            // written out as `completed`.
            if cancellation.is_cancelled() {
                break false;
            }
            if let Some(granted) = self.take_budget_decision(run_id).await {
                break granted;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let wait = remaining.min(BUDGET_DB_POLL_INTERVAL);
            tokio::select! {
                biased;
                _ = cancellation.cancelled_signal() => break false,
                _ = self.budget_notify.notified() => continue,
                _ = tokio::time::sleep(wait) => continue,
            }
        };
        self.budget_decisions.lock().await.remove(&run_id);
        if cancellation.is_cancelled() {
            return None;
        }
        Some(granted)
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
            .map_err(|e| format!("resolve_budget update for run {run_id} failed: {e:#}"))?;
        if res.rows_affected() == 0 {
            return Err(format!(
                "run {run_id} is not waiting for a budget extension"
            ));
        }
        self.budget_decisions.lock().await.insert(run_id, granted);
        // Wake the `await_budget_extension` loop immediately; the DB row it
        // also wrote stays as the audit/fallback copy.
        self.budget_notify.notify_waiters();
        Ok(())
    }

    async fn take_budget_decision(&self, run_id: RunId) -> Option<bool> {
        if let Some(granted) = self.budget_decisions.lock().await.get(&run_id).copied() {
            return Some(granted);
        }
        // A query failure reads as "no answer yet" (the waiter keeps polling),
        // but it must be logged: treating a locked database as a missing answer
        // delayed every budget prompt by the full timeout.
        let row: Option<(String,)> =
            sqlx::query_as::<_, (String,)>("SELECT data FROM runs WHERE id = ?")
                .bind(run_id.to_string())
                .fetch_optional(self.storage.pool())
                .await
                .map_err(|err| {
                    tracing::warn!(%run_id, %err, "take_budget_decision query failed; treating as no answer");
                    err
                })
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
        args: &serde_json::Value,
    ) -> RunToolCall {
        // The wire string is decoded once by the caller — the stagnation
        // detector and the verification ledger read the very same object — so
        // this takes the decoded form and copies it into the record it has to
        // persist.
        let arguments = args.clone();
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

        // One read of the registry for the whole call, taken as a snapshot
        // rather than a guard. The gate used to take the lock twice here and a
        // third time across the tool execution itself, so a thirty-second
        // `terminal` call held the read lock that an MCP reconnect's write lock
        // is queued behind for the entire command.
        let registry = self.tools.read().await.clone();

        // Permission gate: a stored rule may allow, deny, or prompt.
        //
        // The fallback is per-*call*, not per tool: `files` is one tool but its
        // read and write actions warrant different answers, and an unknown tool
        // is always treated as sensitive rather than optimistically allowed.
        let pattern_target = registry.pattern_target_for(&tool_name, &arguments);
        let default_decision = match registry.get(&tool_name) {
            Some(tool) => tool.decision_for(&arguments),
            None => mixd_tools::permission::PermissionDecision::Ask,
        };
        let permission_ctx = mixd_tools::execute::PermissionContext {
            workspace_id: Some(request.workspace_id),
            pattern_target: pattern_target.as_deref(),
        };
        // A stored rule still wins, so a user can lock a read-only tool down or
        // silence a prompt for a mutating one. Scope and pattern now participate
        // in the lookup: a workspace-scoped deny beats a global allow, and a
        // pattern-scoped rule only matches when the tool exposes a matching
        // target.
        let decision = self
            .executor
            .resolve_permission(&tool_name, &permission_ctx, default_decision)
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
                .request_approval(
                    run_id,
                    &tool_call_id,
                    &tool_name,
                    arguments.clone(),
                    Some(request.workspace_id),
                    pattern_target.clone(),
                )
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
                _ = tokio::time::sleep(ttl) => ApprovalOutcome::Deny,
            };
            self.set_run_state(run_id, RunStatus::Running, serde_json::json!({}))
                .await;

            // Every outcome has to retire the card: the reducer only clears
            // `timeline.approval` on this event, so a denial that emitted
            // nothing left a live "approve / deny" card on screen for the rest
            // of the run.
            let granted = matches!(outcome, ApprovalOutcome::Approve { .. });
            self.emit(
                run_id,
                "tool_approval_resolved",
                serde_json::json!({
                    "tool_call_id": tool_call_id,
                    "approval_id": record.approval_id,
                    "granted": granted,
                }),
            )
            .await;
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
            additional_paths: request.additional_paths.clone(),
            approval_token,
        };
        let outcome = registry.dispatch(ctx, &tool_name, arguments.clone()).await;

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
    ///
    /// Takes the outgoing history by value so the loop does not have to copy
    /// the conversation a second time to hand it over.
    async fn run_turn(
        &self,
        run_id: RunId,
        provider: &dyn Provider,
        request: &RunRequest,
        messages: Vec<Message>,
        system: &str,
    ) -> anyhow::Result<TurnResult> {
        let req = ProviderRequest {
            model: request.wire_model().to_string(),
            messages,
            system: if system.trim().is_empty() {
                None
            } else {
                Some(system.to_string())
            },
            tools: Arc::clone(&request.tools),
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

        // A connection-level failure (bad key, DNS, refused socket) used to
        // escape through `?` before any terminal event was emitted, so the row
        // stayed `running` and the UI spun forever. It is reported exactly like
        // an in-stream error.
        let stream = match provider.stream(req).await {
            Ok(stream) => stream,
            Err(err) => {
                tracing::error!(%run_id, provider = provider.id(), %err, "provider stream failed to open");
                return Ok(TurnResult {
                    text: String::new(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    usage: None,
                    finish_reason: None,
                    error: Some(err.to_string()),
                });
            }
        };
        let mut s = stream;
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut usage: Option<Usage> = None;
        let mut finish_reason: Option<String> = None;
        let mut error: Option<String> = None;
        let mut pending: HashMap<String, (String, String)> = HashMap::new();
        let mut order: Vec<String> = Vec::new();

        // `cancelled_signal` is polled alongside the stream so a Run cancelled
        // while the provider is generating stops promptly instead of after the
        // whole response has arrived.
        let cancellation = self.cancellation_of(run_id).await;
        loop {
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled_signal() => {
                    error = Some("run cancelled".to_string());
                    break;
                }
                ev = futures_util::StreamExt::next(&mut s) => ev,
            };
            let Some(ev) = next else {
                break;
            };
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
                    if !order.contains(&id) {
                        order.push(id.clone());
                    }
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

        // Fallback heal for leaked DeepSeek DSML markup. The OpenAI-compatible
        // adapter already filters the live stream, but any other transport —
        // or a turn assembled from cached history — can still carry raw
        // `<｜DSML｜…>` tags in `text`. Parse them back into tool calls so the
        // turn executes instead of stalling, and never persist raw tags.
        let mut tool_calls: Vec<AssistantToolCall> = order
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
        if error.is_none() && mixd_providers::dsml::contains_dsml(&text) {
            let (cleaned, healed) = mixd_providers::dsml::sanitize_assistant_text(&text);
            if !healed.is_empty() {
                tracing::warn!(
                    %run_id,
                    healed = healed.len(),
                    "healed leaked DSML tool call(s) from turn text"
                );
                for call in healed {
                    tool_calls.push(AssistantToolCall {
                        id: mixd_providers::dsml::synthesize_call_id(),
                        kind: "function".to_string(),
                        function: AssistantToolCallFn {
                            name: call.name,
                            arguments: call.arguments,
                        },
                    });
                }
            }
            text = cleaned;
        }

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
///
/// A query failure or corrupt blob reads as "no override" with a warn, never
/// as a silent default: without the log a locked database looked identical to
/// a run with no stored budget.
async fn budget_snapshot(run_id: &RunId, engine: &RunEngine) -> Option<serde_json::Value> {
    let row: Option<(String,)> =
        sqlx::query_as::<_, (String,)>("SELECT data FROM runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_optional(engine.storage.pool())
            .await
            .map_err(|err| {
                tracing::warn!(%run_id, %err, "budget_snapshot query failed; using mode default");
                err
            })
            .unwrap_or(None);
    let data: serde_json::Value = row
        .and_then(|(raw,)| {
            serde_json::from_str(&raw)
                .map_err(|err| {
                    tracing::warn!(%run_id, %err, "budget_snapshot saw a corrupt runs.data blob; using mode default");
                })
                .ok()
        })
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
fn strip_head(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .skip_while(|m| {
            m.role == "system"
                && (m.content.starts_with("<task_state>")
                    || m.content.starts_with("<compressed_history>"))
        })
        .cloned()
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
        tools: Arc::new(Vec::new()),
        temperature: Some(SUMMARIZER_TEMPERATURE),
        top_p: None,
        max_tokens: Some(SUMMARIZER_MAX_TOKENS),
        reasoning: None,
        stream: true,
    };
    let mut stream = match provider.stream(req).await {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(?err, "summarize_turns: provider stream failed");
            return None;
        }
    };
    let mut out = String::new();
    while let Some(ev) = futures_util::StreamExt::next(&mut stream).await {
        match ev {
            StreamEvent::TextDelta(delta) => out.push_str(&delta),
            StreamEvent::Finished { .. } => break,
            StreamEvent::Error(e) => {
                tracing::warn!(?e, "summarize_turns: stream error");
                return None;
            }
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
    run_id: RunId,
) -> String {
    let mut head = task_state.render();
    let checklist = todos.render(run_id).await;
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

#[cfg(test)]
mod wire_model_tests {
    use super::wire_model_for as wire_model;

    #[test]
    fn drops_the_provider_prefix_a_discovered_model_id_carries() {
        assert_eq!(wire_model("p1:gpt-4o", "p1"), "gpt-4o");
        assert_eq!(
            wire_model(
                "4f234c67-eeee-4457-8140:deepseek-ai/deepseek-v4.1",
                "4f234c67-eeee-4457-8140"
            ),
            "deepseek-ai/deepseek-v4.1"
        );
    }

    /// A local-runtime tag carries its own colon, and providers key on the tag.
    /// Splitting off anything but the exact `<provider_id>:` prefix keeps it
    /// whole.
    #[test]
    fn keeps_a_model_name_that_contains_its_own_colon() {
        assert_eq!(wire_model("p1:llama3.1:8b", "p1"), "llama3.1:8b");
        assert_eq!(
            wire_model("p1:qwen2.5-coder:0.5b-instruct-q4_K_M", "p1"),
            "qwen2.5-coder:0.5b-instruct-q4_K_M"
        );
    }

    #[test]
    fn passes_through_an_id_that_carries_no_prefix() {
        assert_eq!(wire_model("gpt-4o", "p1"), "gpt-4o");
        assert_eq!(wire_model("llama3.1:8b", "p1"), "llama3.1:8b");
    }

    /// The prefix has to match in full. A provider id that is only a prefix of
    /// the model id (`a` against `abc:model`) must not be cut at all, or the
    /// adapter would be handed `bc:model`.
    #[test]
    fn does_not_cut_a_partial_provider_id_match() {
        assert_eq!(wire_model("abc:model", "a"), "abc:model");
        assert_eq!(wire_model("ab:model", "abc"), "ab:model");
    }

    /// A blank provider id used to be handed straight through by the resume
    /// path (`unwrap_or_default()`); it must not strip a leading colon.
    #[test]
    fn is_a_no_op_without_a_provider_id() {
        assert_eq!(wire_model(":leading", ""), ":leading");
        assert_eq!(wire_model("gpt-4o", ""), "gpt-4o");
    }
}

#[cfg(test)]
mod execute_tests {
    use super::*;

    struct MockProvider {
        id: mixd_providers::ProviderId,
    }

    #[async_trait::async_trait]
    impl mixd_providers::Provider for MockProvider {
        fn id(&self) -> &mixd_providers::ProviderId {
            &self.id
        }
        fn label(&self) -> &str {
            "mock"
        }
        fn capabilities(&self) -> mixd_providers::ProviderCapabilities {
            mixd_providers::ProviderCapabilities {
                vision: false,
                tool_calling: true,
                reasoning: false,
                streaming: true,
            }
        }
        async fn stream(&self, _req: ProviderRequest) -> anyhow::Result<mixd_providers::StreamBox> {
            let events = vec![
                StreamEvent::TextDelta("hello".to_string()),
                StreamEvent::Finished {
                    finish_reason: "stop".to_string(),
                    usage: None,
                },
            ];
            Ok(Box::pin(futures_util::stream::iter(events)))
        }
    }

    /// The execute loop completes against a stub provider with no tools.
    #[tokio::test]
    async fn execute_with_mock_provider() {
        let dir = std::env::temp_dir().join(format!("mixd-engine-test-{}", uuid::Uuid::new_v4()));
        let storage = Arc::new(Storage::open(&dir).await.expect("open storage"));
        let secrets = Arc::new(mixd_storage::SecretStore::new());
        let engine = Arc::new(RunEngine::with_storage(storage, secrets));
        engine
            .providers
            .register(
                "mock".to_string(),
                Arc::new(MockProvider {
                    id: "mock".to_string(),
                }),
            )
            .await;
        let request = RunRequest {
            run_id: RunId::new_v4(),
            conversation_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: None,
            additional_paths: Vec::new(),
            mode: Mode::Chat,
            model_id: "mock:model".to_string(),
            provider_id: "mock".to_string(),
            system: None,
            verify_command: None,
            format_command: None,
            context_window: 0,
            temperature: None,
            reasoning: None,
            max_tokens: None,
            temporary: true,
            tools: Arc::new(Vec::new()),
            history: Vec::new(),
            user_message: "hi".to_string(),
            resume_from: None,
        };
        let outcome = engine.execute(request).await.expect("execute");
        assert_eq!(outcome.status, RunStatus::Completed);
        assert_eq!(outcome.text, "hello");
        assert_eq!(outcome.steps, 1);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
