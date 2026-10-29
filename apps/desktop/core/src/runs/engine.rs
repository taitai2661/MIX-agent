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

mod event_bus;
mod execute;
mod execute_budget;
mod execute_tools;
mod execute_turn;
mod finish;
mod helpers;
mod query;

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
    /// Files the Run is about to change, captured before the first change so
    /// the whole Run can be reverted. In memory: a restart drops the undo
    /// history, which is the honest lifetime for "undo what just happened".
    pub snapshots: Arc<tokio::sync::Mutex<HashMap<RunId, Vec<FileSnapshot>>>>,
    /// Content fingerprints of files the Run has read, so an edit can refuse
    /// to apply to content the model never saw.
    pub read_fingerprints:
        Arc<tokio::sync::Mutex<HashMap<RunId, HashMap<std::path::PathBuf, u64>>>>,
    /// Mid-Run user messages waiting to be folded into the next turn.
    ///
    /// A coding agent that can only be stopped, never redirected, throws away
    /// every step it has already taken when the user spots a wrong assumption.
    /// The queue is drained at each turn boundary rather than interrupted into
    /// a streaming completion, so a provider request is never cut mid-token.
    pub steering: Arc<tokio::sync::Mutex<HashMap<RunId, Vec<String>>>>,
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

/// A file's content before a Run first changed it.
///
/// `previous: None` means the file did not exist, so reverting deletes it.
#[derive(Debug, Clone)]
pub struct FileSnapshot {
    pub path: std::path::PathBuf,
    pub previous: Option<Vec<u8>>,
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
            steering: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            snapshots: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            read_fingerprints: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
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
        // Filled by `list_runs` from the persisted events: the count lives in
        // one row per failed tool result, not in the run's data blob.
        failed_tool_count: 0,
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
    /// How long a local runtime should keep the model resident after this
    /// request. Only the native Ollama transport sends it; a cloud provider
    /// ignores it. `None` leaves the runtime's own default in force.
    pub keep_alive: Option<String>,
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
            keep_alive: None,
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

#[cfg(test)]
mod concurrency_tests {
    use super::helpers::can_run_concurrently;

    #[test]
    fn local_reads_are_parallelised() {
        for args in [
            serde_json::json!({ "action": "read", "path": "a.rs" }),
            serde_json::json!({ "action": "list", "path": "." }),
        ] {
            assert!(can_run_concurrently("files", &args));
        }
        assert!(can_run_concurrently("grep", &serde_json::json!({})));
        assert!(can_run_concurrently("glob", &serde_json::json!({})));
        assert!(can_run_concurrently("symbols", &serde_json::json!({})));
        assert!(can_run_concurrently(
            "git",
            &serde_json::json!({ "action": "diff" })
        ));
        assert!(can_run_concurrently(
            "knowledge",
            &serde_json::json!({ "action": "search" })
        ));
    }

    #[test]
    fn writes_and_network_calls_stay_serial() {
        assert!(!can_run_concurrently(
            "files",
            &serde_json::json!({ "action": "write", "path": "a.rs" })
        ));
        assert!(!can_run_concurrently("edit", &serde_json::json!({})));
        assert!(!can_run_concurrently("terminal", &serde_json::json!({})));
        assert!(!can_run_concurrently("web_fetch", &serde_json::json!({})));
        assert!(!can_run_concurrently(
            "workspace_check",
            &serde_json::json!({})
        ));
    }
}
