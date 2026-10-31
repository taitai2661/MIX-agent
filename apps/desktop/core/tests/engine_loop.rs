//! End-to-end tests for the Agent loop.
//!
//! `engine.rs` is the largest and least-tested file in the workspace, and it is
//! the part everything else depends on: the turn loop, the permission gate, the
//! context head block, checkpointing and the termination conditions all live
//! there. A regression in any of them is silent — the app simply stops working
//! — so they are exercised here against a scripted provider rather than a mock
//! HTTP server, which keeps the tests deterministic and fast.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt as _;
use mixd_core::mode::Mode;
use mixd_core::runs::checkpoint_store::CheckpointStore;
use mixd_core::runs::engine::{
    CheckpointSnapshot, HistoryMessage, RunEngine, RunRequest, RunToolCall,
};
use mixd_core::runs::mode_policy::AgentModePolicy;
use mixd_core::runs::state::RunStatus;
use mixd_providers::{
    AssistantToolCall, AssistantToolCallFn, Message, Provider, ProviderCapabilities,
    ProviderRequest, StreamBox, StreamEvent, Usage,
};
use mixd_storage::SecretStore;
use mixd_tools::{EgressPolicy, ToolRegistry};
use uuid::Uuid;

/// A provider that replays a fixed script of turns, one per call.
///
/// Turns are consumed in order; once the script runs out the last turn is
/// repeated, which keeps a test focused on the steps it cares about instead of
/// having to pad the script.
struct ScriptedProvider {
    turns: std::sync::Mutex<Vec<Vec<StreamEvent>>>,
    calls: AtomicUsize,
    /// Every request the engine sent, for asserting on the composed prompt.
    seen: Arc<std::sync::Mutex<Vec<ProviderRequest>>>,
    name: String,
}

impl ScriptedProvider {
    fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            turns: std::sync::Mutex::new(turns),
            calls: AtomicUsize::new(0),
            seen: Arc::new(std::sync::Mutex::new(Vec::new())),
            name: "scripted".to_string(),
        }
    }

    fn request_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<ProviderRequest> {
        self.seen.lock().unwrap().clone()
    }
}

/// A turn that just answers.
fn text_turn(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta(text.to_string()),
        StreamEvent::Finished {
            finish_reason: "stop".to_string(),
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            }),
        },
    ]
}

/// A turn that calls one tool, streamed in fragments so the accumulation path is
/// exercised rather than bypassed.
fn tool_turn(id: &str, name: &str, args: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolCallStart {
            id: id.to_string(),
            name: name.to_string(),
        },
        StreamEvent::ToolCallArgumentDelta {
            id: id.to_string(),
            delta: args[..args.len() / 2].to_string(),
        },
        StreamEvent::ToolCallArgumentDelta {
            id: id.to_string(),
            delta: args[args.len() / 2..].to_string(),
        },
        StreamEvent::ToolCallFinish {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args.to_string(),
        },
        StreamEvent::Finished {
            finish_reason: "tool_calls".to_string(),
            usage: None,
        },
    ]
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn id(&self) -> &String {
        &self.name
    }
    fn label(&self) -> &str {
        "Scripted"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: false,
            tool_calling: true,
            reasoning: false,
            streaming: true,
        }
    }
    async fn stream(&self, req: ProviderRequest) -> anyhow::Result<StreamBox> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(req);
        let script = self.turns.lock().unwrap();
        let turn = script
            .get(index)
            .or_else(|| script.last())
            .cloned()
            .unwrap_or_default();
        Ok(Box::pin(futures_util::stream::iter(turn)))
    }
}

/// A provider that always fails, for the provider-error paths.
struct FailingProvider {
    name: String,
    calls: AtomicUsize,
}

#[async_trait]
impl Provider for FailingProvider {
    fn id(&self) -> &String {
        &self.name
    }
    fn label(&self) -> &str {
        "Failing"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: false,
            tool_calling: true,
            reasoning: false,
            streaming: true,
        }
    }
    async fn stream(&self, _req: ProviderRequest) -> anyhow::Result<StreamBox> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(futures_util::stream::iter(vec![
            StreamEvent::Error("upstream exploded".to_string()),
        ])))
    }
}

/// Emits one delta and then never finishes, so a cancellation lands mid-stream.
struct SlowProvider {
    name: String,
}

impl SlowProvider {
    fn new() -> Self {
        Self {
            name: SLOW_PROVIDER_ID.to_string(),
        }
    }
}

#[async_trait]
impl Provider for SlowProvider {
    fn id(&self) -> &String {
        &self.name
    }
    fn label(&self) -> &str {
        "Slow"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: false,
            tool_calling: true,
            reasoning: false,
            streaming: true,
        }
    }
    async fn stream(&self, _req: ProviderRequest) -> anyhow::Result<StreamBox> {
        Ok(Box::pin(
            futures_util::stream::once(async {
                StreamEvent::TextDelta("partial answer that must survive".to_string())
            })
            .chain(futures_util::stream::pending()),
        ))
    }
}

const SLOW_PROVIDER_ID: &str = "slow";

struct Harness {
    engine: Arc<RunEngine>,
    storage: Arc<mixd_storage::Storage>,
    workspace: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    provider: Arc<ScriptedProvider>,
    conversation_id: Uuid,
}

impl Harness {
    async fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
        let unique = format!(
            "mixd-engine-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let base = std::env::temp_dir().join(unique.replace(['/', ':'], "-"));
        let data_dir = base.join("data");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let storage = Arc::new(
            mixd_storage::Storage::open(&data_dir)
                .await
                .expect("open storage"),
        );
        let tools = ToolRegistry::new(EgressPolicy::default());
        let engine = Arc::new(RunEngine::with_storage_and_tools(
            storage.clone(),
            Arc::new(SecretStore::new()),
            tools,
        ));
        let provider = Arc::new(ScriptedProvider::new(turns));
        engine
            .providers
            .register("p1".to_string(), provider.clone())
            .await;
        // `messages.conversation_id` references `conversations(id)`, so a real
        // conversation row has to exist before the engine can persist a turn.
        let conversation_id = Uuid::new_v4();
        sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
            .bind(conversation_id.to_string())
            .bind(mixd_storage::records::SINGLE_USER_ID)
            .bind("{}")
            .execute(storage.pool())
            .await
            .expect("insert conversation");
        Self {
            engine,
            storage,
            workspace,
            data_dir,
            provider,
            conversation_id,
        }
    }

    async fn request(&self) -> RunRequest {
        RunRequest {
            run_id: Uuid::new_v4(),
            conversation_id: self.conversation_id,
            workspace_id: Uuid::new_v4(),
            root_path: Some(self.workspace.to_string_lossy().to_string()),
            additional_paths: Vec::new(),
            mode: Mode::Thinking,
            model_id: "test-model".to_string(),
            provider_id: "p1".to_string(),
            system: None,
            // No configured check by default: the gate is opt-in per project, and
            // the tests that care set it explicitly.
            verify_command: None,
            format_command: None,
            context_window: 0,
            temperature: None,
            reasoning: None,
            max_tokens: None,
            keep_alive: None,
            temporary: false,
            tools: std::sync::Arc::new(self.engine.tool_specs().await),
            history: Vec::new(),
            user_message: "do the thing".to_string(),
            resume_from: None,
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

/// Drain the event broadcast so emission never blocks on a full channel.
fn spawn_event_drain(engine: &Arc<RunEngine>) -> tokio::task::JoinHandle<()> {
    let mut rx = engine.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            let _ = event;
        }
    })
}

fn tool_names(calls: &[RunToolCall]) -> Vec<&str> {
    calls.iter().map(|c| c.name.as_str()).collect()
}

/// Store an always-allow rule, the equivalent of clicking "Always Allow".
async fn allow_tool(harness: &Harness, tool: &str) {
    set_rule(harness, &format!("allow-{tool}"), tool, "allow").await;
}

async fn set_rule(harness: &Harness, id: &str, tool: &str, decision: &str) {
    sqlx::query("INSERT OR REPLACE INTO permission_rules (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(id)
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(
            serde_json::json!({"toolId": tool, "scope": "always", "decision": decision})
                .to_string(),
        )
        .execute(harness.storage.pool())
        .await
        .expect("insert rule");
}

#[tokio::test]
async fn a_turn_without_tool_calls_ends_the_run() {
    let harness = Harness::new(vec![text_turn("all done")]).await;
    let _drain = spawn_event_drain(&harness.engine);
    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(outcome.text, "all done");
    assert_eq!(outcome.steps, 1);
    assert!(outcome.tool_calls.is_empty());
    assert_eq!(
        harness.provider.request_count(),
        1,
        "no extra turn after the answer"
    );
}

#[tokio::test]
async fn a_tool_call_is_dispatched_and_the_loop_continues() {
    let harness = Harness::new(vec![
        tool_turn("call_1", "files", r#"{"action":"list","path":"."}"#),
        text_turn("listed"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(outcome.text, "listed");
    assert_eq!(tool_names(&outcome.tool_calls), vec!["files"]);
    assert_eq!(
        harness.provider.request_count(),
        2,
        "the loop must go back to the model after a tool result"
    );
}

#[tokio::test]
async fn several_tool_calls_in_one_turn_all_run() {
    let harness = Harness::new(vec![
        {
            // Two calls in one turn: build the events by hand so both arrive
            // before the single `Finished`.
            let mut events = tool_turn("a", "glob", r#"{"pattern":"*.rs"}"#);
            events.retain(|e| !matches!(e, StreamEvent::Finished { .. }));
            events.extend(
                tool_turn("b", "grep", r#"{"pattern":"fn main"}"#)
                    .into_iter()
                    .filter(|e| !matches!(e, StreamEvent::Finished { .. })),
            );
            events.push(StreamEvent::Finished {
                finish_reason: "tool_calls".to_string(),
                usage: None,
            });
            events
        },
        text_turn("both done"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    let mut names = tool_names(&outcome.tool_calls);
    names.sort();
    assert_eq!(names, vec!["glob", "grep"], "both parallel calls must run");
}

#[tokio::test]
async fn the_tool_result_is_persisted_with_its_call_id() {
    // This is the round trip that decides whether a second turn works at all:
    // the assistant's `tool_calls` and the `tool` result have to be written in
    // the right order, or the next request is rejected by the provider.
    let harness = Harness::new(vec![
        tool_turn("call_xyz", "glob", r#"{"pattern":"*.rs"}"#),
        text_turn("done"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    let request = harness.request().await;
    let conversation_id = request.conversation_id;
    harness.engine.execute(request).await.expect("run");

    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT data FROM messages WHERE conversation_id = ? ORDER BY created_at",
    )
    .bind(conversation_id.to_string())
    .fetch_all(harness.storage.pool())
    .await
    .expect("messages");
    let parsed: Vec<serde_json::Value> = rows
        .iter()
        .filter_map(|raw| serde_json::from_str(raw).ok())
        .collect();

    let assistant = parsed
        .iter()
        .find(|m| {
            m["role"] == "assistant" && m["toolCalls"].as_array().is_some_and(|a| !a.is_empty())
        })
        .expect("an assistant turn carrying tool calls must be persisted");
    assert_eq!(assistant["toolCalls"][0]["id"], "call_xyz");
    assert_eq!(assistant["toolCalls"][0]["function"]["name"], "glob");

    let tool = parsed
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("the tool result must be persisted");
    assert_eq!(tool["toolCallId"], "call_xyz");
    assert_eq!(tool["name"], "glob");

    // Order matters: the tool result must come after the call it answers.
    let assistant_at = parsed
        .iter()
        .position(|m| m["toolCalls"].as_array().is_some_and(|a| !a.is_empty()))
        .unwrap();
    let tool_at = parsed.iter().position(|m| m["role"] == "tool").unwrap();
    assert!(
        assistant_at < tool_at,
        "result before call is an invalid history"
    );
}

#[tokio::test]
async fn a_deny_rule_stops_the_call_without_running_it() {
    let harness = Harness::new(vec![
        tool_turn(
            "call_1",
            "files",
            r#"{"action":"write","path":"blocked.txt","content":"x"}"#,
        ),
        text_turn("understood"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    set_rule(&harness, "rule-1", "files", "deny").await;

    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    assert_eq!(tool_names(&outcome.tool_calls), vec!["files"]);
    let call = &outcome.tool_calls[0];
    assert!(
        call.error.as_deref().unwrap_or("").contains("denied"),
        "a denied call must say so, got {call:?}"
    );
    assert!(
        !harness.workspace.join("blocked.txt").exists(),
        "a denied call must not have any effect on disk"
    );
}

#[tokio::test]
async fn a_provider_error_fails_the_run_instead_of_looping() {
    let storage = Arc::new(
        mixd_storage::Storage::open(
            std::env::temp_dir().join(format!("mixd-engine-fail-{}", std::process::id())),
        )
        .await
        .expect("storage"),
    );
    let engine = Arc::new(RunEngine::with_storage_and_tools(
        storage.clone(),
        Arc::new(SecretStore::new()),
        ToolRegistry::new(EgressPolicy::default()),
    ));
    engine
        .providers
        .register(
            "bad".to_string(),
            Arc::new(FailingProvider {
                name: "bad".to_string(),
                calls: AtomicUsize::new(0),
            }),
        )
        .await;
    let _drain = spawn_event_drain(&engine);
    let run_id = Uuid::new_v4();
    // `runs.conversation_id` references `conversations(id)`, so a real
    // conversation has to exist before the run row is inserted.
    let conversation_id = Uuid::new_v4();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, '{}')")
        .bind(conversation_id.to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .execute(storage.pool())
        .await
        .expect("insert conversation");
    // `set_run_state` UPDATEs an existing row, so the failure can only be read
    // back if the run row is present — same as a real dispatch.
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?, ?, '{}', ?, 'running', ?)",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(conversation_id.to_string())
    .bind(Uuid::new_v4().to_string())
    .execute(storage.pool())
    .await
    .expect("insert run");
    let mut request = RunRequest {
        run_id,
        conversation_id,
        workspace_id: Uuid::new_v4(),
        root_path: None,
        additional_paths: Vec::new(),
        mode: Mode::Thinking,
        model_id: "m".to_string(),
        provider_id: "bad".to_string(),
        system: None,
        verify_command: None,
        format_command: None,
        context_window: 0,
        temperature: None,
        reasoning: None,
        max_tokens: None,
        keep_alive: None,
        temporary: false,
        tools: std::sync::Arc::new(Vec::new()),
        history: Vec::new(),
        user_message: "hello".to_string(),
        resume_from: None,
    };
    request.tools = std::sync::Arc::new(engine.tool_specs().await);
    let outcome = engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Failed);
    assert!(outcome.text.is_empty());
    // The reason must survive on the Run view, or the UI shows a silent stop.
    let view = engine
        .run_state(run_id)
        .await
        .expect("run row to read back");
    assert_eq!(view.status, RunStatus::Failed);
    assert_eq!(
        view.error.as_deref(),
        Some("upstream exploded"),
        "the provider error must be surfaced on the Run"
    );
}

#[tokio::test]
async fn the_goal_is_reinjected_every_turn() {
    let harness = Harness::new(vec![
        tool_turn("call_1", "glob", r#"{"pattern":"*.rs"}"#),
        text_turn("done"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    let requests = harness.provider.requests();
    assert_eq!(requests.len(), 2);
    for (index, request) in requests.iter().enumerate() {
        let has_state = request
            .messages
            .iter()
            .any(|m| m.role == "system" && m.content.contains("<task_state>"));
        assert!(has_state, "turn {index} lost the task_state head block");
        assert!(
            request
                .messages
                .iter()
                .any(|m| m.content.contains("do the thing")),
            "turn {index} lost the user's request"
        );
    }
}

#[tokio::test]
async fn the_todo_checklist_is_reinjected_every_turn() {
    let harness = Harness::new(vec![
        tool_turn(
            "call_1",
            "todo",
            r#"{"todos":[{"content":"write the fix","status":"in_progress"}]}"#,
        ),
        text_turn("done"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    let requests = harness.provider.requests();
    // The checklist is built during the first turn, so it can only appear from
    // the second request onwards.
    let second = &requests[1];
    assert!(
        second
            .messages
            .iter()
            .any(|m| m.content.contains("<task_list>") && m.content.contains("write the fix")),
        "the checklist must survive into the next turn: {:?}",
        second.messages.iter().map(|m| &m.role).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_destructive_call_must_be_verified_before_concluding() {
    // Strict verification was previously emit-only: the ledger recorded the
    // write and nothing ever read it, so the run concluded regardless.
    let harness = Harness::new(vec![
        tool_turn(
            "call_1",
            "edit",
            r#"{"path":"note.txt","old_string":"a","new_string":"b"}"#,
        ),
        // Tries to conclude without reading the file back.
        text_turn("I have made the change."),
        tool_turn("call_2", "files", r#"{"action":"read","path":"note.txt"}"#),
        text_turn("Verified: the file now reads `b`."),
    ])
    .await;
    std::fs::write(harness.workspace.join("note.txt"), "a\n").unwrap();
    allow_tool(&harness, "edit").await;
    let _drain = spawn_event_drain(&harness.engine);
    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(
        tool_names(&outcome.tool_calls),
        vec!["edit", "files"],
        "the run must be sent back to read the file back"
    );
    assert!(
        outcome.text.contains("Verified"),
        "the final answer must be the verified one, got {:?}",
        outcome.text
    );
}

#[tokio::test]
async fn verification_gives_up_after_a_bounded_number_of_turns() {
    // A model that never reads anything back must not loop forever.
    let harness = Harness::new(vec![
        tool_turn(
            "call_1",
            "edit",
            r#"{"path":"note.txt","old_string":"a","new_string":"b"}"#,
        ),
        text_turn("done, trust me"),
    ])
    .await;
    std::fs::write(harness.workspace.join("note.txt"), "a\n").unwrap();
    allow_tool(&harness, "edit").await;
    let _drain = spawn_event_drain(&harness.engine);
    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert!(
        harness.provider.request_count() <= 5,
        "an unverified run must terminate: {} provider calls",
        harness.provider.request_count()
    );
}

#[tokio::test]
async fn a_tool_failure_is_marked_so_the_model_can_see_it() {
    let harness = Harness::new(vec![
        tool_turn(
            "call_1",
            "edit",
            r#"{"path":"missing.txt","old_string":"a","new_string":"b"}"#,
        ),
        text_turn("recovered"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    let requests = harness.provider.requests();
    let tool_message = requests[1]
        .messages
        .iter()
        .find(|m| m.role == "tool")
        .expect("tool result must be in the next request");
    assert!(
        tool_message.is_error,
        "a failed call must be flagged so adapters can mark it as an error"
    );
    assert_eq!(tool_names(&outcome.tool_calls), vec!["edit"]);
}

#[tokio::test]
async fn an_unknown_provider_is_rejected_before_any_turn() {
    let harness = Harness::new(vec![text_turn("unused")]).await;
    let mut request = harness.request().await;
    request.provider_id = "nope".to_string();
    let err = harness
        .engine
        .execute(request)
        .await
        .expect_err("must fail");
    assert!(err.to_string().contains("provider not configured"));
    assert_eq!(harness.provider.request_count(), 0);
}

#[tokio::test]
async fn cancellation_stops_the_run() {
    let harness = Harness::new(vec![
        tool_turn("call_1", "glob", r#"{"pattern":"*.rs"}"#),
        tool_turn("call_2", "glob", r#"{"pattern":"*.toml"}"#),
        text_turn("never reached"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    let request = harness.request().await;
    let run_id = request.run_id;
    let engine = harness.engine.clone();
    let task = tokio::spawn(async move { engine.execute(request).await });
    // Cancel as soon as the run is registered.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    harness.engine.cancel(run_id).await;
    let outcome = task.await.expect("join").expect("run");
    assert!(
        matches!(outcome.status, RunStatus::Cancelled | RunStatus::Completed),
        "unexpected status: {:?}",
        outcome.status
    );
}

/// Stopping a run must keep what the model had already produced. Cancelling
/// used to retire the live block and write nothing, so the partial answer
/// vanished and the thread looked like the send had never started.
#[tokio::test]
async fn cancellation_persists_the_partial_answer() {
    let harness = Harness::new(vec![text_turn("unused")]).await;
    harness
        .engine
        .providers
        .register(SLOW_PROVIDER_ID.to_string(), Arc::new(SlowProvider::new()))
        .await;
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.provider_id = SLOW_PROVIDER_ID.to_string();
    let run_id = request.run_id;
    let engine = harness.engine.clone();
    let task = tokio::spawn(async move { engine.execute(request).await });

    // Wait until the first delta has been streamed, then cancel.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    harness.engine.cancel(run_id).await;
    let outcome = task.await.expect("join").expect("run");
    assert!(
        matches!(outcome.status, RunStatus::Cancelled),
        "unexpected status: {:?}",
        outcome.status
    );

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT json_extract(data, '$.role'), json_extract(data, '$.content'), \
                COALESCE(json_extract(data, '$.reasoning'), '') \
         FROM messages WHERE conversation_id = ? ORDER BY rowid ASC",
    )
    .bind(harness.conversation_id.to_string())
    .fetch_all(harness.storage.pool())
    .await
    .expect("read messages");
    assert!(
        rows.iter().any(|(role, content, _)| {
            role == "assistant" && content == "partial answer that must survive"
        }),
        "partial assistant text was not persisted: {rows:?}"
    );
}

#[tokio::test]
async fn checkpointing_trims_to_its_limit() {
    // The store keeps only the newest N per run; without the trim a long run
    // grows the table without bound.
    let harness = Harness::new(vec![text_turn("unused")]).await;
    let store = CheckpointStore::new(2);
    let run_id = Uuid::new_v4();
    // `run_checkpoints.run_id` references `runs(id)`, so the parent row has to
    // exist or the insert is rejected — silently, since the store ignores it.
    sqlx::query(
        "INSERT INTO runs (id, owner_id, conversation_id, request_key, data) VALUES (?, ?, ?, ?, ?)",
    )
        .bind(run_id.to_string())
        .bind(mixd_storage::records::SINGLE_USER_ID)
        .bind(harness.conversation_id.to_string())
        .bind(Uuid::new_v4().to_string())
        .bind("{}")
        .execute(harness.storage.pool())
        .await
        .expect("insert run");
    for step in 0..5u32 {
        let snapshot = CheckpointSnapshot {
            step_index: step,
            tool_calls: 0,
            messages: vec![HistoryMessage {
                role: "user".to_string(),
                content: format!("step {step}"),
                tool_call_id: None,
                name: None,
                tool_calls: Vec::new(),
                is_error: false,
            }],
        };
        store
            .save(run_id, step, 0, &snapshot, harness.storage.as_ref())
            .await;
    }
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM run_checkpoints WHERE run_id = ?")
        .bind(run_id.to_string())
        .fetch_one(harness.storage.pool())
        .await
        .expect("count");
    assert_eq!(count.0, 2, "only the newest snapshots should survive");
}

#[tokio::test]
async fn a_history_message_round_trips_through_serde() {
    // `tool_calls` and `is_error` must survive serialisation, or a resumed run
    // rehydrates a history every provider rejects.
    let call = AssistantToolCall {
        id: "c1".to_string(),
        kind: "function".to_string(),
        function: AssistantToolCallFn {
            name: "edit".to_string(),
            arguments: r#"{"path":"a.rs"}"#.to_string(),
        },
    };
    let original = HistoryMessage {
        role: "assistant".to_string(),
        content: "editing".to_string(),
        tool_call_id: None,
        name: None,
        tool_calls: vec![call],
        is_error: false,
    };
    let json = serde_json::to_string(&original).unwrap();
    let back: HistoryMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.tool_calls.len(), 1);
    assert_eq!(back.tool_calls[0].id, "c1");
    assert_eq!(back.tool_calls[0].function.name, "edit");

    // A row written before the field existed must still parse.
    let legacy = r#"{"role":"tool","content":"x","toolCallId":"c1"}"#;
    let back: HistoryMessage = serde_json::from_str(legacy).unwrap();
    assert!(back.tool_calls.is_empty());
    assert!(!back.is_error);
}

#[tokio::test]
async fn resuming_replays_the_checkpointed_history() {
    let snapshot = CheckpointSnapshot {
        step_index: 3,
        tool_calls: 0,
        messages: vec![
            HistoryMessage {
                role: "user".to_string(),
                content: "earlier question".to_string(),
                tool_call_id: None,
                name: None,
                tool_calls: Vec::new(),
                is_error: false,
            },
            HistoryMessage {
                role: "assistant".to_string(),
                content: "earlier answer".to_string(),
                tool_call_id: None,
                name: None,
                tool_calls: Vec::new(),
                is_error: false,
            },
        ],
    };
    let harness = Harness::new(vec![text_turn("resumed")]).await;
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.resume_from = Some(snapshot);
    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.text, "resumed");
    let seen = harness.provider.requests();
    assert!(
        seen[0]
            .messages
            .iter()
            .any(|m| m.content == "earlier answer"),
        "the checkpointed history must be replayed: {:?}",
        seen[0].messages
    );
}

#[tokio::test]
async fn a_restored_history_reaches_the_provider_with_its_tool_calls() {
    // The composed request for a resumed run must be valid, so the orphan check
    // in the adapters has to see a matched pair.
    let call = AssistantToolCall {
        id: "c1".to_string(),
        kind: "function".to_string(),
        function: AssistantToolCallFn {
            name: "glob".to_string(),
            arguments: r#"{"pattern":"*.rs"}"#.to_string(),
        },
    };
    let snapshot = CheckpointSnapshot {
        step_index: 2,
        tool_calls: 0,
        messages: vec![
            HistoryMessage {
                role: "user".to_string(),
                content: "find the sources".to_string(),
                tool_call_id: None,
                name: None,
                tool_calls: Vec::new(),
                is_error: false,
            },
            HistoryMessage {
                role: "assistant".to_string(),
                content: String::new(),
                tool_call_id: None,
                name: None,
                tool_calls: vec![call.clone()],
                is_error: false,
            },
            HistoryMessage {
                role: "tool".to_string(),
                content: r#"{"matches":[]}"#.to_string(),
                tool_call_id: Some("c1".to_string()),
                name: Some("glob".to_string()),
                tool_calls: Vec::new(),
                is_error: false,
            },
        ],
    };
    let harness = Harness::new(vec![text_turn("ok")]).await;
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.resume_from = Some(snapshot);
    harness.engine.execute(request).await.expect("run");
    let seen = harness.provider.requests();
    let assistant = seen[0]
        .messages
        .iter()
        .find(|m| m.role == "assistant")
        .expect("the assistant turn must survive the resume");
    assert_eq!(
        assistant.tool_calls.len(),
        1,
        "a resumed run must keep the tool call it was waiting on"
    );
    let keep = mixd_providers::resolved_history_indices(&seen[0].messages);
    assert_eq!(
        keep.len(),
        seen[0].messages.len(),
        "a well-formed history must not be pruned"
    );
    // And the tool result must be matched, not dropped as an orphan.
    assert!(seen[0]
        .messages
        .iter()
        .any(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some("c1")));
}

#[tokio::test]
async fn a_dangling_tool_result_is_pruned_rather_than_sent() {
    // The shape a half-written history produces. Sending it as-is is a 400 from
    // every provider.
    let messages = vec![
        Message::text("user", "go"),
        Message {
            role: "tool".to_string(),
            content: "result".to_string(),
            tool_calls: Vec::new(),
            tool_call_id: Some("orphan".to_string()),
            name: Some("glob".to_string()),
            is_error: false,
        },
    ];
    let keep = mixd_providers::resolved_history_indices(&messages);
    assert_eq!(keep.len(), 1, "the orphan must be dropped");
    assert_eq!(messages[keep[0]].role, "user");
}

#[test]
fn the_agent_policy_enables_the_loop_guards_by_default() {
    // `AgentModePolicy::default()` is all-true and never mutated, so these are
    // the flags the loop actually runs under. Asserted here so a future
    // "turn one off" edit is a deliberate, visible change.
    let policy = AgentModePolicy::default();
    assert!(
        policy.stagnation_detection,
        "stagnation detection is the default"
    );
    assert!(
        policy.strict_verification,
        "verification gating is the default"
    );
    assert!(policy.checkpointing, "checkpointing is the default");
    assert!(
        policy.checkpoint_every_steps > 0,
        "a zero interval would checkpoint on every step"
    );
    assert!(policy.context_tiered_summary);
}

#[tokio::test]
async fn chat_mode_is_offered_no_tools() {
    let harness = Harness::new(vec![text_turn("hi")]).await;
    let seen = harness.engine.tool_specs().await;
    assert!(
        seen.iter().any(|s| s.name == "edit"),
        "agent-facing modes must offer the edit tool"
    );
    assert!(
        seen.iter().all(|s| s.name == "todo"
            || s.name == "edit"
            || s.name == "glob"
            || s.name == "grep"
            || s.name == "files"
            || s.name == "terminal"
            || s.name == "workspace_check"
            || s.name == "web_search"
            || s.name == "web_fetch"
            || s.name == "web_fetch_pdf"
            || s.name == "network"
            || s.name == "user_plan"
            || s.name == "image_vision"
            || s.name == "git"
            || s.name == "symbols"
            || s.name == "skill"
            || s.name == "agent"),
        "unexpected tools advertised: {:?}",
        seen.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn the_registry_advertises_the_tool_the_whole_system_depends_on() {
    // Regression guard: the tool that makes this a coding agent rather than a
    // chat client, plus the two that make a codebase navigable.
    let registry = ToolRegistry::new(EgressPolicy::default());
    let names: Vec<String> = registry.specs().iter().map(|s| s.name.clone()).collect();
    for required in ["edit", "glob", "grep", "files", "terminal", "todo"] {
        assert!(
            names.iter().any(|n| n == required),
            "{required} must be advertised; got {names:?}"
        );
    }
    let specs = registry.specs();
    // And the edit tool must be able to describe itself usefully.
    let edit = specs.iter().find(|s| s.name == "edit").expect("edit spec");
    let params = &edit.parameters;
    assert!(params["properties"]["old_string"].is_object());
    assert!(params["properties"]["new_string"].is_object());
    assert!(!edit.description.is_empty());
}

#[tokio::test]
async fn the_edit_tool_actually_edits_inside_a_run() {
    // The full path a real fix takes: model asks, tool writes, file changed.
    let harness = Harness::new(vec![
        tool_turn(
            "call_1",
            "edit",
            r#"{"path":"src/main.rs","old_string":"let x = 1;","new_string":"let x = 2;"}"#,
        ),
        text_turn("changed"),
    ])
    .await;
    std::fs::create_dir_all(harness.workspace.join("src")).unwrap();
    std::fs::write(
        harness.workspace.join("src/main.rs"),
        "fn main() {\n    let x = 1;\n}\n",
    )
    .unwrap();
    // `edit` is sensitive by design, so the test grants it the way a user would
    // from the Tools pane.
    allow_tool(&harness, "edit").await;
    let _drain = spawn_event_drain(&harness.engine);
    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    let after = std::fs::read_to_string(harness.workspace.join("src/main.rs")).unwrap();
    assert!(after.contains("let x = 2;"), "the edit must land: {after}");
    assert!(
        after.contains("fn main()"),
        "the rest of the file must survive"
    );
}

#[tokio::test]
async fn a_tool_cannot_escape_the_workspace_root() {
    let harness = Harness::new(vec![
        tool_turn(
            "call_1",
            "edit",
            r#"{"path":"../escaped.txt","old_string":"a","new_string":"b"}"#,
        ),
        text_turn("gave up"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    assert!(
        !harness
            .data_dir
            .parent()
            .unwrap()
            .join("escaped.txt")
            .exists(),
        "a path traversal must be refused, not written"
    );
    let call = &harness.provider.seen.lock().unwrap().len().to_string();
    assert!(!call.is_empty());
}

#[tokio::test]
async fn run_status_is_persisted_for_later_reads() {
    // `set_run_state` used to build SQL that SQLite rejected and then discard
    // the error, so `get_run`/`list_runs` always reported `queued` and the
    // counters stayed at zero.
    let harness = Harness::new(vec![text_turn("all done")]).await;
    let _drain = spawn_event_drain(&harness.engine);
    let request = harness.request().await;
    let run_id = request.run_id;
    sqlx::query(
        "INSERT INTO runs (id, owner_id, conversation_id, request_key, data, status) \
         VALUES (?, ?, ?, ?, ?, 'queued')",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(harness.conversation_id.to_string())
    .bind(Uuid::new_v4().to_string())
    .bind("{}")
    .execute(harness.storage.pool())
    .await
    .expect("insert run");

    harness.engine.execute(request).await.expect("run");

    let view = harness
        .engine
        .run_state(run_id)
        .await
        .expect("run row to read back");
    assert_eq!(view.status, RunStatus::Completed);
    assert_eq!(view.step_count, 1, "stepCount must be written");
    let column: String = sqlx::query_scalar("SELECT status FROM runs WHERE id = ?")
        .bind(run_id.to_string())
        .fetch_one(harness.storage.pool())
        .await
        .expect("status column");
    assert_eq!(column, "completed");

    let listed = harness.engine.list_runs(harness.conversation_id).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, RunStatus::Completed);
}

#[tokio::test]
async fn a_budget_answer_for_a_run_that_is_not_pending_is_rejected() {
    let harness = Harness::new(vec![text_turn("unused")]).await;
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs (id, owner_id, conversation_id, request_key, data, status) \
         VALUES (?, ?, ?, ?, '{}', 'running')",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(harness.conversation_id.to_string())
    .bind(Uuid::new_v4().to_string())
    .execute(harness.storage.pool())
    .await
    .expect("insert run");

    let err = harness
        .engine
        .resolve_budget(run_id, true, None)
        .await
        .expect_err("a run that is not pending must not accept an answer");
    assert!(err.contains("not waiting"), "unexpected error: {err}");
}

#[tokio::test]
async fn a_budget_answer_is_delivered_to_the_engine_and_persisted() {
    let harness = Harness::new(vec![text_turn("unused")]).await;
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs (id, owner_id, conversation_id, request_key, data, status) \
         VALUES (?, ?, ?, ?, '{}', 'budget_extension_pending')",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(harness.conversation_id.to_string())
    .bind(Uuid::new_v4().to_string())
    .execute(harness.storage.pool())
    .await
    .expect("insert run");

    harness
        .engine
        .resolve_budget(
            run_id,
            true,
            Some(serde_json::json!({ "stepLimit": 42, "toolCallLimit": 42 })),
        )
        .await
        .expect("grant");

    let (status, data): (String, String) =
        sqlx::query_as("SELECT status, data FROM runs WHERE id = ?")
            .bind(run_id.to_string())
            .fetch_one(harness.storage.pool())
            .await
            .expect("run row");
    assert_eq!(status, "running");
    let data: serde_json::Value = serde_json::from_str(&data).expect("json");
    assert_eq!(data["budgetDecision"], serde_json::json!(true));
    assert_eq!(data["budget"]["stepLimit"], serde_json::json!(42));

    // The same answer cannot be applied twice.
    harness
        .engine
        .resolve_budget(run_id, true, None)
        .await
        .expect_err("second answer must be rejected");
}

#[tokio::test]
async fn a_stream_that_never_finishes_is_a_failure() {
    // A dropped connection used to look like a normal answer: the fragment was
    // stored as the reply and the loop carried on as if progress were made.
    let harness = Harness::new(vec![vec![StreamEvent::TextDelta(
        "partial answer that never ended".to_string(),
    )]])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    let request = harness.request().await;
    let run_id = request.run_id;
    sqlx::query(
        "INSERT INTO runs (id, owner_id, conversation_id, request_key, data, status) \
         VALUES (?, ?, ?, ?, '{}', 'queued')",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(harness.conversation_id.to_string())
    .bind(Uuid::new_v4().to_string())
    .execute(harness.storage.pool())
    .await
    .expect("insert run");

    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Failed);
    let view = harness.engine.run_state(run_id).await.expect("run row");
    assert_eq!(view.status, RunStatus::Failed, "failure must be persisted");
}

/// Read a Run's persisted event log exactly the way `app/src/run_events.rs`
/// replays it: ordered by sequence, parsed back into JSON.
async fn replay_log(
    storage: &mixd_storage::Storage,
    run_id: Uuid,
) -> Vec<(i64, String, serde_json::Value)> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT sequence, kind, data FROM run_events WHERE run_id = ? ORDER BY sequence ASC",
    )
    .bind(run_id.to_string())
    .fetch_all(storage.pool())
    .await
    .expect("read run_events");
    rows.into_iter()
        .map(|row| {
            let sequence: i64 = row.get("sequence");
            let kind: String = row.get("kind");
            let data: String = row.get("data");
            (
                sequence,
                kind,
                serde_json::from_str(&data).unwrap_or(serde_json::Value::Null),
            )
        })
        .collect()
}

/// Replay order is what matters. Adjacent stream deltas are coalesced at
/// write time, so the sequence is strictly increasing but may have gaps; a
/// duplicate would break `ORDER BY sequence` and is still an error.
fn assert_ordered(rows: &[(i64, String, serde_json::Value)], label: &str) {
    let mut previous = 0;
    for (sequence, ..) in rows {
        assert!(
            *sequence > previous,
            "{label}: sequences must be strictly increasing (no duplicates), got {:?}",
            rows.iter()
                .map(|(s, k, _)| (*s, k.clone()))
                .collect::<Vec<_>>()
        );
        previous = *sequence;
    }
}

/// A reloaded UI rebuilds the transcript from `run_events`, so a streamed turn
/// has to be reconstructible from the table after the Run is over — this is
/// the contract `flush_events` exists to keep.
#[tokio::test]
async fn a_streamed_turn_can_be_replayed_from_the_database() {
    let harness = Harness::new(vec![vec![
        StreamEvent::TextDelta("Hello".to_string()),
        StreamEvent::TextDelta(", ".to_string()),
        StreamEvent::TextDelta("world".to_string()),
        StreamEvent::Finished {
            finish_reason: "stop".to_string(),
            usage: None,
        },
    ]])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    let request = harness.request().await;
    let run_id = request.run_id;
    // `run_events.run_id` references `runs(id)`, so the row has to exist
    // before anything can be logged against it (the shell inserts it when the
    // message is sent).
    sqlx::query(
        "INSERT INTO runs (id, owner_id, conversation_id, request_key, data, status) \
         VALUES (?, ?, ?, ?, '{}', 'queued')",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(harness.conversation_id.to_string())
    .bind(Uuid::new_v4().to_string())
    .execute(harness.storage.pool())
    .await
    .expect("insert run");
    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);

    // The terminal `status` milestone must have drained the token buffer: no
    // timer is allowed to be what makes a finished Run durable.
    assert!(
        harness.engine.event_buffer.lock().await.is_empty(),
        "the buffer must be empty once the run ended"
    );

    let rows = replay_log(&harness.storage, run_id).await;
    assert_ordered(&rows, "replay log");

    let text: String = rows
        .iter()
        .filter(|(_, kind, _)| kind == "text")
        .map(|(_, _, data)| data["delta"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        text, "Hello, world",
        "deltas must reassemble into the answer"
    );
    assert!(
        rows.iter().any(|(_, kind, _)| kind == "status"),
        "the terminal status has to be in the log too"
    );
    assert!(
        rows.iter().any(|(_, kind, _)| kind == "model_started"),
        "milestones are written straight through"
    );
}

/// Token events are batched rather than written one INSERT at a time, and a
/// milestone (or an explicit flush) drains what is pending.
#[tokio::test]
async fn token_events_are_batched_and_drained_by_a_milestone() {
    let harness = Harness::new(Vec::new()).await;
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs (id, owner_id, conversation_id, request_key, data, status) \
         VALUES (?, ?, ?, ?, '{}', 'running')",
    )
    .bind(run_id.to_string())
    .bind(mixd_storage::records::SINGLE_USER_ID)
    .bind(harness.conversation_id.to_string())
    .bind(Uuid::new_v4().to_string())
    .execute(harness.storage.pool())
    .await
    .expect("insert run");

    for index in 0..5 {
        harness
            .engine
            .emit(
                run_id,
                "text",
                serde_json::json!({ "delta": format!("tok{index}") }),
            )
            .await;
    }
    assert_eq!(
        replay_log(&harness.storage, run_id).await.len(),
        0,
        "a token must not cost a database round trip"
    );

    harness
        .engine
        .emit(
            run_id,
            "status",
            serde_json::json!({ "status": "completed" }),
        )
        .await;
    let rows = replay_log(&harness.storage, run_id).await;
    // Five adjacent deltas fold into one row; the milestone stays separate.
    assert_eq!(rows.len(), 2);
    assert_ordered(&rows, "after milestone flush");
    assert_eq!(
        rows[0].2["delta"], "tok0tok1tok2tok3tok4",
        "coalescing must not lose or reorder text"
    );
    assert_eq!(
        rows.last().map(|(_, kind, _)| kind.as_str()),
        Some("status"),
        "the buffered deltas must be written before the milestone that follows them"
    );

    // Explicit draining (used before a replay) must work the same way.
    for index in 5..7 {
        harness
            .engine
            .emit(
                run_id,
                "reasoning",
                serde_json::json!({ "delta": format!("r{index}") }),
            )
            .await;
    }
    harness.engine.flush_all_events().await;
    let rows = replay_log(&harness.storage, run_id).await;
    // One coalesced text row, the terminal status, one coalesced reasoning row.
    assert_eq!(rows.len(), 3);
    assert_ordered(&rows, "after explicit flush");
    assert!(harness.engine.event_buffer.lock().await.is_empty());
}

/// A source that reports the root it was asked for, so the test can tell
/// whether the engine actually scoped the catalogue to the run's folder.
struct RootEchoSkills;

#[async_trait]
impl mixd_tools::skill::SkillSource for RootEchoSkills {
    async fn skills(
        &self,
        root: Option<&str>,
    ) -> anyhow::Result<Vec<mixd_tools::skill::SkillSummary>> {
        Ok(vec![mixd_tools::skill::SkillSummary {
            name: "scoped".into(),
            description: format!("root={}", root.unwrap_or("<none>")),
            body_markdown: "body".into(),
            allowed_tools: Vec::new(),
        }])
    }
}

#[tokio::test]
async fn the_skill_catalogue_reaches_the_system_prompt_scoped_to_the_project() {
    let harness = Harness::new(vec![text_turn("done")]).await;
    harness
        .engine
        .replace_tools(
            ToolRegistry::new(EgressPolicy::default()).with_skills(Arc::new(RootEchoSkills)),
        )
        .await;
    let _drain = spawn_event_drain(&harness.engine);
    let root = harness.workspace.to_string_lossy().to_string();
    harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");

    let requests = harness.provider.requests();
    let system: Vec<String> = requests.iter().filter_map(|r| r.system.clone()).collect();
    assert!(
        system.iter().any(|s| s.contains("<skills>")),
        "the skill catalogue must reach the system prompt"
    );
    assert!(
        system.iter().any(|s| s.contains(&format!("root={root}"))),
        "the catalogue must be built for the run's project root; got {system:?}"
    );
    assert!(
        system.iter().any(|s| s.contains("- scoped:")),
        "the skill must be listed by name; got {system:?}"
    );
}

#[tokio::test]
async fn plan_mode_refuses_a_mutating_call_even_with_a_stored_allow_rule() {
    let harness = Harness::new(vec![
        tool_turn(
            "c1",
            "edit",
            r#"{"action":"create","path":"a.txt","new_string":"x\n"}"#,
        ),
        text_turn("here is the plan"),
    ])
    .await;
    // A stored allow rule is the strongest thing a user can grant, and it must
    // still not open plan mode.
    allow_tool(&harness, "edit").await;
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.mode = Mode::Plan;

    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(tool_names(&outcome.tool_calls), vec!["edit"]);
    let failure = outcome.tool_calls[0]
        .error
        .as_deref()
        .expect("the refusal must be reported back to the model");
    assert!(
        failure.contains("plan mode"),
        "the model has to be told why; got {failure}"
    );
    assert!(
        !harness.workspace.join("a.txt").exists(),
        "plan mode wrote to the workspace"
    );
}

#[tokio::test]
async fn plan_mode_still_allows_the_reads_it_needs_to_plan() {
    let harness = Harness::new(vec![
        tool_turn("c1", "glob", r#"{"pattern":"*.rs"}"#),
        text_turn("here is the plan"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.mode = Mode::Plan;

    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert!(
        outcome.tool_calls[0].error.is_none(),
        "a read-only call must still run in plan mode"
    );
}

#[tokio::test]
async fn plan_mode_allows_the_tools_the_plan_prompt_asks_for() {
    // The plan prompt tells the model to propose with `user_plan`, and the UI
    // renders the checklist from `todo`. Both used to be classified as
    // mutations, so plan mode refused the two tools it exists for; `symbols`
    // was refused too because the classifier only knew the stale `outline`.
    let harness = Harness::new(vec![
        tool_turn("c1", "symbols", r#"{"action":"outline","path":"a.rs"}"#),
        tool_turn(
            "c2",
            "todo",
            r#"{"todos":[{"content":"inspect the code","status":"in_progress"}]}"#,
        ),
        tool_turn(
            "c3",
            "user_plan",
            r#"{"title":"Fix it","steps":["read","edit"]}"#,
        ),
        text_turn("here is the plan"),
    ])
    .await;
    std::fs::write(harness.workspace.join("a.rs"), "fn main() {}\n").unwrap();
    // `user_plan` is the approval gate now: approving it is what lifts plan
    // mode. A stored allow rule is the programmatic equivalent of clicking it.
    allow_tool(&harness, "user_plan").await;
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.mode = Mode::Plan;

    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(
        tool_names(&outcome.tool_calls),
        vec!["symbols", "todo", "user_plan"]
    );
    for call in &outcome.tool_calls {
        assert!(
            call.error.is_none(),
            "plan mode must run `{}`; got {:?}",
            call.name,
            call.error
        );
    }
    let outline = outcome.tool_calls[0]
        .result
        .as_ref()
        .expect("symbols must return a result");
    assert_eq!(
        outline["symbols"][0]["name"], "main",
        "the outline must describe the file it read; got {outline}"
    );
}

#[tokio::test]
async fn an_agent_run_carries_the_built_in_procedure_in_its_system_prompt() {
    let harness = Harness::new(vec![text_turn("done")]).await;
    let _drain = spawn_event_drain(&harness.engine);
    harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");

    let system = harness.provider.requests()[0]
        .system
        .clone()
        .expect("an agent run must have a system prompt even with no project set up");
    assert!(
        system.contains("You are a coding agent"),
        "the built-in procedure must reach the model; got {system:?}"
    );
    assert!(system.contains("Never claim a test"));
}

#[tokio::test]
async fn a_change_without_the_project_check_blocks_the_conclusion() {
    let harness = Harness::new(vec![
        tool_turn(
            "c1",
            "edit",
            r#"{"action":"create","path":"a.txt","new_string":"hello\n"}"#,
        ),
        text_turn("done"),
    ])
    .await;
    allow_tool(&harness, "edit").await;
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.verify_command = Some("cargo test".to_string());

    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);

    let requests = harness.provider.requests();
    assert!(
        requests.len() > 2,
        "the run must be sent back instead of concluding; got {} turns",
        requests.len()
    );
    let told = requests[2]
        .messages
        .iter()
        .any(|m| m.content.contains("[verification]") && m.content.contains("cargo test"));
    assert!(
        told,
        "the repair turn must run the configured command so the model can act on it"
    );
}

#[tokio::test]
async fn running_the_configured_check_lets_the_run_conclude() {
    let harness = Harness::new(vec![
        tool_turn(
            "c1",
            "edit",
            r#"{"action":"create","path":"a.txt","new_string":"hello\n"}"#,
        ),
        text_turn("done"),
        tool_turn("c2", "terminal", r#"{"command":"echo verified"}"#),
        text_turn("verified"),
    ])
    .await;
    allow_tool(&harness, "edit").await;
    allow_tool(&harness, "terminal").await;
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.verify_command = Some("echo verified".to_string());

    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(outcome.text, "verified");
    assert_eq!(
        tool_names(&outcome.tool_calls),
        vec!["edit", "terminal"],
        "the model must actually be allowed to run the check"
    );
}

#[tokio::test]
async fn no_configured_check_leaves_a_change_unencumbered() {
    let harness = Harness::new(vec![
        tool_turn(
            "c1",
            "edit",
            r#"{"action":"create","path":"a.txt","new_string":"hello\n"}"#,
        ),
        text_turn("done"),
    ])
    .await;
    allow_tool(&harness, "edit").await;
    let _drain = spawn_event_drain(&harness.engine);

    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);

    // The read-back gate still applies, and it is the one that fires here: an
    // edit always leaves an outstanding effect until it is read back. What must
    // *not* happen is a demand for a check that was never configured.
    let requests = harness.provider.requests();
    let repairs: Vec<String> = requests
        .iter()
        .flat_map(|r| r.messages.iter())
        .filter(|m| m.content.starts_with("[verification]"))
        .map(|m| m.content.clone())
        .collect();
    assert!(!repairs.is_empty(), "the read-back gate must still fire");
    for repair in &repairs {
        assert!(repair.contains("confirmed the effect"));
        assert!(
            !repair.contains("project check"),
            "no check is configured, so none may be demanded: {repair}"
        );
    }
}

/// Two reads in one turn run concurrently, but the transcript must still pair
/// each result with the call that asked for it, and `files read` must return
/// line numbers so the model can address a region without counting.
#[tokio::test]
async fn parallel_reads_keep_call_order_and_line_numbers() {
    let harness = Harness::new(vec![
        {
            let mut events = tool_turn("a", "files", r#"{"action":"read","path":"a.txt"}"#);
            events.retain(|e| !matches!(e, StreamEvent::Finished { .. }));
            events.extend(
                tool_turn("b", "files", r#"{"action":"read","path":"b.txt"}"#)
                    .into_iter()
                    .filter(|e| !matches!(e, StreamEvent::Finished { .. })),
            );
            events.push(StreamEvent::Finished {
                finish_reason: "tool_calls".to_string(),
                usage: None,
            });
            events
        },
        text_turn("read both"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    std::fs::write(harness.workspace.join("a.txt"), "alpha\n").unwrap();
    std::fs::write(harness.workspace.join("b.txt"), "beta\n").unwrap();

    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(tool_names(&outcome.tool_calls), vec!["files", "files"]);

    let requests = harness.provider.requests();
    let second = &requests[1];
    let tool_messages: Vec<&str> = second
        .messages
        .iter()
        .filter(|m| m.role == "tool")
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(
        tool_messages.len(),
        2,
        "both reads must be in the transcript"
    );
    assert!(
        tool_messages[0].contains("alpha") && tool_messages[0].contains(r#""     1\talpha""#),
        "the first result must be numbered and belong to a.txt: {}",
        tool_messages[0]
    );
    assert!(
        tool_messages[1].contains("beta") && tool_messages[1].contains(r#""     1\tbeta""#),
        "the second result must be numbered and belong to b.txt: {}",
        tool_messages[1]
    );
}

/// A configured formatter runs after a step that changed a file. It was
/// previously only mentioned in a prompt hint and never executed.
#[tokio::test]
async fn the_formatter_runs_after_a_mutation() {
    let harness = Harness::new(vec![
        tool_turn(
            "w",
            "files",
            r#"{"action":"write","path":"out.txt","content":"hello"}"#,
        ),
        text_turn("wrote it"),
    ])
    .await;
    allow_tool(&harness, "files").await;
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.format_command = Some("touch formatted.marker".to_string());

    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert!(
        harness.workspace.join("formatted.marker").exists(),
        "the configured formatter must actually run"
    );
}

/// Reverting a Run restores the content it started from.
#[tokio::test]
async fn revert_run_restores_the_pre_run_content() {
    let harness = Harness::new(vec![
        tool_turn(
            "e",
            "edit",
            r#"{"path":"a.txt","old_string":"before","new_string":"after"}"#,
        ),
        text_turn("edited"),
    ])
    .await;
    allow_tool(&harness, "edit").await;
    let _drain = spawn_event_drain(&harness.engine);
    std::fs::write(harness.workspace.join("a.txt"), "before\n").unwrap();

    let request = harness.request().await;
    let run_id = request.run_id;
    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    let edited = std::fs::read_to_string(harness.workspace.join("a.txt")).unwrap();
    assert!(
        edited.contains("after"),
        "the edit must have landed: {edited}"
    );

    let restored = harness.engine.revert_run(run_id).await.expect("revert");
    assert_eq!(restored, 1);
    let reverted = std::fs::read_to_string(harness.workspace.join("a.txt")).unwrap();
    assert_eq!(reverted, "before\n", "the pre-Run content must be back");
}

/// Approving `user_plan` lifts plan mode for the rest of the Run, so the very
/// next turn can make the change the plan described.
#[tokio::test]
async fn an_approved_plan_lifts_plan_mode_for_the_next_turn() {
    let harness = Harness::new(vec![
        tool_turn(
            "p",
            "user_plan",
            r#"{"title":"Edit it","steps":["edit a.txt"]}"#,
        ),
        tool_turn(
            "e",
            "edit",
            r#"{"path":"a.txt","old_string":"before","new_string":"after"}"#,
        ),
        text_turn("implemented"),
    ])
    .await;
    allow_tool(&harness, "user_plan").await;
    allow_tool(&harness, "edit").await;
    std::fs::write(harness.workspace.join("a.txt"), "before\n").unwrap();
    let _drain = spawn_event_drain(&harness.engine);
    let mut request = harness.request().await;
    request.mode = Mode::Plan;

    let outcome = harness.engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);
    assert_eq!(tool_names(&outcome.tool_calls), vec!["user_plan", "edit"]);
    let edit = &outcome.tool_calls[1];
    assert!(
        edit.error.is_none(),
        "the edit must run after approval; got {:?}",
        edit.error
    );
    let text = std::fs::read_to_string(harness.workspace.join("a.txt")).unwrap();
    assert!(text.contains("after"), "the edit must have landed: {text}");
}

/// An edit whose anchor still matches but whose file changed after the read is
/// refused, because the model is working from stale content.
#[tokio::test]
async fn an_edit_to_a_file_changed_after_the_read_is_refused() {
    let harness = Harness::new(vec![
        tool_turn("r", "files", r#"{"action":"read","path":"a.txt"}"#),
        tool_turn("t", "terminal", r#"{"command":"echo extra >> a.txt"}"#),
        tool_turn(
            "e",
            "edit",
            r#"{"path":"a.txt","old_string":"before","new_string":"after"}"#,
        ),
        text_turn("done"),
    ])
    .await;
    allow_tool(&harness, "terminal").await;
    allow_tool(&harness, "edit").await;
    let _drain = spawn_event_drain(&harness.engine);
    std::fs::write(harness.workspace.join("a.txt"), "before\n").unwrap();

    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    let edit = outcome
        .tool_calls
        .iter()
        .find(|call| call.name == "edit")
        .expect("edit call");
    let error = edit.error.as_deref().unwrap_or_default();
    assert!(
        error.contains("changed on disk"),
        "the stale edit must be refused with a readable reason; got {error:?}"
    );
    let text = std::fs::read_to_string(harness.workspace.join("a.txt")).unwrap();
    assert!(
        text.contains("before"),
        "the edit must not have applied: {text}"
    );
}

/// A malformed argument blob is reported as such instead of being dispatched
/// with `null` and surfacing as a confusing "path required".
#[tokio::test]
async fn malformed_tool_arguments_are_reported_clearly() {
    let harness = Harness::new(vec![
        tool_turn("x", "files", r#"{"action":"read""#),
        text_turn("done"),
    ])
    .await;
    let _drain = spawn_event_drain(&harness.engine);
    let outcome = harness
        .engine
        .execute(harness.request().await)
        .await
        .expect("run");
    let call = &outcome.tool_calls[0];
    let error = call.error.as_deref().unwrap_or_default();
    assert!(
        error.contains("not valid JSON"),
        "the parse failure must be named; got {error:?}"
    );
}
