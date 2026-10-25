//! Spec §6 post-response auto formation (自動形成): the shared extraction
//! prompt, the durable job queue, the retry state machine, the filtering rules
//! and the synchronous「覚えて／忘れて」intake.
//!
//! Everything runs against a temporary SQLite file and a scripted provider, so
//! the suite never touches the network.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use mixd_core::memory::{
    apply_extraction, auto_formation_enabled, explicit_candidate, handle_explicit,
    parse_extraction, ExplicitOutcome, ExtractedMemory, MemoryCreate, MemoryService,
    EXTRACTION_PROMPT,
};
use mixd_core::memory_jobs::{
    due_jobs, enqueue, JobOutcome, JobRunner, EXISTING_LIMIT, MAX_ATTEMPTS,
};
use mixd_core::mode::Mode;
use mixd_core::runs::engine::{ProviderRegistry, RunEngine, RunRequest};
use mixd_core::runs::state::RunStatus;
use mixd_providers::{
    Provider, ProviderCapabilities, ProviderRequest, StreamBox, StreamEvent, Usage,
};
use mixd_storage::records::SINGLE_USER_ID;
use mixd_storage::{SecretStore, Storage};
use mixd_tools::{EgressPolicy, ToolRegistry};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// What the fake provider does when it is called.
enum Script {
    Reply(String),
    Error(String),
    /// Answers with a stream that never emits: the §6 90 s ceiling, injected
    /// as milliseconds through [`JobRunner::with_timeout`].
    Hang,
}

struct FakeProvider {
    name: String,
    script: Mutex<Script>,
    calls: AtomicUsize,
    last_request: Mutex<Option<ProviderRequest>>,
}

impl FakeProvider {
    fn new(script: Script) -> Self {
        Self {
            name: "p1".to_string(),
            script: Mutex::new(script),
            calls: AtomicUsize::new(0),
            last_request: Mutex::new(None),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_request(&self) -> ProviderRequest {
        self.last_request
            .lock()
            .expect("request lock")
            .clone()
            .expect("the provider was never called")
    }
}

#[async_trait]
impl Provider for FakeProvider {
    fn id(&self) -> &String {
        &self.name
    }

    fn label(&self) -> &str {
        "Fake"
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_request.lock().expect("request lock") = Some(req);
        let script = self.script.lock().expect("script lock");
        match &*script {
            Script::Reply(text) => Ok(Box::pin(futures_util::stream::iter(vec![
                StreamEvent::TextDelta(text.clone()),
                StreamEvent::Finished {
                    finish_reason: "stop".to_string(),
                    usage: Some(Usage::default()),
                },
            ]))),
            Script::Error(message) => Ok(Box::pin(futures_util::stream::iter(vec![
                StreamEvent::Error(message.clone()),
            ]))),
            Script::Hang => Ok(Box::pin(futures_util::stream::pending())),
        }
    }
}

/// A temporary data directory that cleans itself up.
struct TempStore {
    storage: Arc<Storage>,
    dir: std::path::PathBuf,
}

impl TempStore {
    async fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "mixd-memory-extraction-{tag}-{}-{:?}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let storage = Arc::new(Storage::open(&dir).await.expect("open storage"));
        Self { storage, dir }
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn insert_conversation(storage: &Storage) -> Uuid {
    let conversation_id = Uuid::new_v4();
    sqlx::query("INSERT INTO conversations (id, owner_id, data) VALUES (?, ?, ?)")
        .bind(conversation_id.to_string())
        .bind(SINGLE_USER_ID)
        .bind("{}")
        .execute(storage.pool())
        .await
        .expect("insert conversation");
    conversation_id
}

/// The `runs` row extraction reads back for `modelId` / `providerId` — the
/// foreign key means it cannot be invented later.
async fn insert_run(storage: &Storage, run_id: &str, conversation_id: Uuid, data: Value) {
    sqlx::query(
        "INSERT INTO runs (id, owner_id, data, conversation_id, status, request_key) \
         VALUES (?, ?, ?, ?, 'queued', ?)",
    )
    .bind(run_id)
    .bind(SINGLE_USER_ID)
    .bind(data.to_string())
    .bind(conversation_id.to_string())
    .bind(format!("key-{run_id}"))
    .execute(storage.pool())
    .await
    .expect("insert run");
}

/// A conversation plus its run, ready for [`enqueue`].
async fn seed_run(storage: &Storage, run_id: &str, data: Value) -> Uuid {
    let conversation_id = insert_conversation(storage).await;
    insert_run(storage, run_id, conversation_id, data).await;
    conversation_id
}

async fn build_runner(storage: Arc<Storage>, script: Script) -> (JobRunner, Arc<FakeProvider>) {
    let provider = Arc::new(FakeProvider::new(script));
    let registry = Arc::new(ProviderRegistry::default());
    registry.register("p1".to_string(), provider.clone()).await;
    (JobRunner::new(storage, registry), provider)
}

/// `status`, `attempts`, `id` for one run's job row.
async fn job_row(storage: &Storage, run_id: &str) -> (String, i64, String) {
    let row =
        sqlx::query("SELECT id, status, attempts FROM memory_processing_jobs WHERE run_id = ?")
            .bind(run_id)
            .fetch_one(storage.pool())
            .await
            .expect("job row");
    (
        row.get("status"),
        row.get("attempts"),
        row.get::<String, _>("id"),
    )
}

async fn job_data(storage: &Storage, run_id: &str) -> Value {
    let raw: String =
        sqlx::query_scalar("SELECT data FROM memory_processing_jobs WHERE run_id = ?")
            .bind(run_id)
            .fetch_one(storage.pool())
            .await
            .expect("job data");
    serde_json::from_str(&raw).expect("job data is JSON")
}

/// Rewind the backoff window so the next tick may claim the job.
async fn make_due(storage: &Storage, run_id: &str) {
    let past = (Utc::now() - ChronoDuration::seconds(2)).to_rfc3339();
    sqlx::query(
        "UPDATE memory_processing_jobs SET data = json_set(data, '$.nextAttemptAt', ?) \
         WHERE run_id = ?",
    )
    .bind(past)
    .bind(run_id)
    .execute(storage.pool())
    .await
    .expect("rewind backoff");
}

async fn set_auto_formation(storage: &Storage, enabled: bool) {
    sqlx::query(
        "INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, ?) \
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
    )
    .bind(SINGLE_USER_ID)
    .bind(json!({ "memory_auto_formation": enabled }).to_string())
    .execute(storage.pool())
    .await
    .expect("write setting");
}

async fn memories(storage: &Storage) -> Vec<String> {
    MemoryService::list(storage, 50)
        .await
        .expect("list memories")
        .into_iter()
        .map(|item| item.content)
        .collect()
}

async fn count_all(storage: &Storage, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(storage.pool())
        .await
        .expect("count")
}

async fn count_for(storage: &Storage, sql: &str, bind: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(storage.pool())
        .await
        .expect("count")
}

// ---------------------------------------------------------------------------
// The shared prompt and the parser
// ---------------------------------------------------------------------------

/// The prompt is a *byte-for-byte* copy of spec §6 — the parity script fails
/// CI on a one-character drift, so pin it here too.
#[test]
fn extraction_prompt_matches_the_spec_verbatim() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let spec_path = {
        let p = manifest.join("../../docs/memory-spec.md");
        if p.exists() {
            p
        } else {
            manifest.join("../../../docs/memory-spec.md")
        }
    };
    let spec = std::fs::read_to_string(&spec_path).expect("read docs/memory-spec.md");
    let marker = "**共通抽出プロンプト**:\n\n```\n";
    let start = spec.find(marker).expect("spec has the prompt marker") + marker.len();
    let end = start + spec[start..].find("\n```").expect("prompt fence closes");
    assert_eq!(
        EXTRACTION_PROMPT,
        &spec[start..end],
        "core::memory::EXTRACTION_PROMPT must be the spec §6 block verbatim"
    );
}

#[test]
fn parse_tolerates_wrappers_defaults_and_refuses_garbage() {
    let reply = "Here you go:\n```json\n\
        {\"memories\":[\
        {\"content\":\"I bike to work every day\"},\
        {\"content\":\"skip this one\",\"action\":\"no-op\"}\
        ]}\n```";
    let items = parse_extraction(reply).expect("parse a fenced reply");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].role, "fact", "a missing role defaults to fact");
    assert_eq!(
        items[0].action, "CREATE",
        "a missing action defaults to CREATE"
    );
    assert_eq!(items[1].action, "NO_OP", "an unknown verb is never stored");

    // Structural problems take the durable retry path rather than silently
    // storing nothing.
    assert!(parse_extraction("no json at all").is_err());
    assert!(parse_extraction(r#"{"memories":[{"content":""}]}"#).is_err());
}

// ---------------------------------------------------------------------------
// Queueing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn enqueue_is_idempotent_redacts_secrets_and_needs_a_run() {
    let store = TempStore::new("enqueue").await;
    let run_id = Uuid::new_v4().to_string();
    seed_run(&store.storage, &run_id, json!({"modelId": "p1:m"})).await;

    assert!(
        enqueue(
            &store.storage,
            &run_id,
            "my password: hunter2",
            "Understood — nothing stored about that.",
        )
        .await
        .expect("enqueue"),
        "the first enqueue inserts a row"
    );
    assert!(
        !enqueue(&store.storage, &run_id, "again", "again")
            .await
            .expect("enqueue"),
        "UNIQUE(run_id) makes the second enqueue a no-op"
    );
    let jobs: i64 = count_all(
        &store.storage,
        "SELECT count(*) FROM memory_processing_jobs",
    )
    .await;
    assert_eq!(jobs, 1);

    let data = job_data(&store.storage, &run_id).await;
    let user = data["userContent"].as_str().expect("user content");
    assert!(
        user.contains("[REDACTED SECRET]"),
        "spec §8 redacts the value: {user}"
    );
    assert!(
        !user.contains("hunter2"),
        "the secret must not be queued: {user}"
    );

    // A run with no `runs` row (a sub-agent turn) queues nothing at all.
    let orphan = Uuid::new_v4().to_string();
    assert!(
        !enqueue(&store.storage, &orphan, "hi", "hello")
            .await
            .expect("enqueue"),
        "a foreign key violation would be worse than no job"
    );
}

#[tokio::test]
async fn due_jobs_lists_only_claimable_work() {
    let store = TempStore::new("due").await;
    for run in ["run-a", "run-b"] {
        seed_run(
            &store.storage,
            run,
            json!({"modelId": "p1:m", "providerId": "p1"}),
        )
        .await;
        assert!(enqueue(&store.storage, run, "hi", "hello")
            .await
            .expect("enqueue"));
    }
    let jobs = due_jobs(&store.storage, 8).await;
    assert_eq!(jobs.len(), 2);

    let (runner, _provider) = build_runner(
        store.storage.clone(),
        Script::Reply(r#"{"memories":[]}"#.to_string()),
    )
    .await;
    let outcome = runner.process_job(&jobs[0].id).await;
    assert!(
        matches!(outcome, JobOutcome::Applied { .. }),
        "an empty reply is a completed extraction: {outcome:?}"
    );
    assert_eq!(
        due_jobs(&store.storage, 8).await.len(),
        1,
        "the finished job leaves the queue"
    );
}

// ---------------------------------------------------------------------------
// One full attempt
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_reply_is_extracted_with_the_spec_call_and_applied() {
    let store = TempStore::new("happy").await;
    let run_id = Uuid::new_v4().to_string();
    seed_run(
        &store.storage,
        &run_id,
        json!({"modelId": "p1:fake-model", "providerId": "p1"}),
    )
    .await;

    // The `existing` list §6 feeds back: top 10 by recency.
    assert_eq!(EXISTING_LIMIT, 10);
    let existing = MemoryService::create(
        &store.storage,
        MemoryCreate {
            title: "coffee".to_string(),
            content: "I drink coffee every morning".to_string(),
            role: "preference".to_string(),
            scope: "user".to_string(),
            source_kind: "user".to_string(),
            task_id: None,
        },
    )
    .await
    .expect("seed memory");

    assert!(enqueue(
        &store.storage,
        &run_id,
        "morning routine?",
        "What do you drink?"
    )
    .await
    .expect("enqueue"));

    let reply = r#"{"memories":[
        {"content":"I drink coffee every morning","role":"preference","action":"REINFORCE","reason":"it repeats"},
        {"content":"My favourite editor is the terminal","role":"fact","action":"CREATE","reason":"stable fact"}]}"#;
    let (runner, provider) =
        build_runner(store.storage.clone(), Script::Reply(reply.to_string())).await;
    let job_id = job_row(&store.storage, &run_id).await.2;

    let outcome = runner.process_job(&job_id).await;
    let (created, reinforced) = match outcome {
        JobOutcome::Applied {
            created,
            reinforced,
        } => (created, reinforced),
        other => panic!("expected the extraction to apply, got {other:?}"),
    };
    assert_eq!(created.len(), 1);
    assert_eq!(
        reinforced,
        vec![existing.id.clone()],
        "an exact match is reinforced"
    );

    // §6 input contract: system = the shared prompt, user = the turn as JSON,
    // `temperature 0`, `max_tokens 800`.
    let request = provider.last_request();
    assert_eq!(
        request.model, "fake-model",
        "the <provider>: prefix is stripped"
    );
    assert_eq!(request.system.as_deref(), Some(EXTRACTION_PROMPT));
    assert_eq!(request.temperature, Some(0.0));
    assert_eq!(request.max_tokens, Some(800));
    assert!(request.stream);
    assert_eq!(request.messages.len(), 1);
    assert_eq!(request.messages[0].role, "user");
    let payload: Value =
        serde_json::from_str(&request.messages[0].content).expect("the user turn is JSON");
    assert_eq!(payload["user"], json!("morning routine?"));
    assert_eq!(payload["assistant"], json!("What do you drink?"));
    let listed = payload["existing"].as_array().expect("existing is a list");
    assert_eq!(listed.len(), 1, "existing holds the stored memories");
    assert_eq!(listed[0], json!("I drink coffee every morning"));

    let (status, attempts, _) = job_row(&store.storage, &run_id).await;
    assert_eq!(status, "completed");
    assert_eq!(attempts, 1);
    let data = job_data(&store.storage, &run_id).await;
    assert_eq!(data["candidateCount"], json!(2));
    assert!(data["completedAt"].is_string());
    assert!(
        data.get("error").is_none(),
        "a completed job carries no error"
    );

    let stored = memories(&store.storage).await;
    assert_eq!(
        stored.len(),
        2,
        "one new memory plus the reinforced one: {stored:?}"
    );
    assert_eq!(
        count_for(
            &store.storage,
            "SELECT count(*) FROM memory_action_events WHERE run_id = ?",
            &run_id
        )
        .await,
        2,
        "spec §1 logs one row per applied candidate"
    );
}

#[tokio::test]
async fn only_three_items_are_written_per_reply() {
    let store = TempStore::new("cap").await;
    let run_id = Uuid::new_v4().to_string();
    seed_run(
        &store.storage,
        &run_id,
        json!({"modelId": "p1:m", "providerId": "p1"}),
    )
    .await;
    assert!(enqueue(&store.storage, &run_id, "u", "a")
        .await
        .expect("enqueue"));

    let reply = r#"{"memories":[
        {"content":"fact one","role":"fact","action":"CREATE","reason":""},
        {"content":"fact two","role":"fact","action":"CREATE","reason":""},
        {"content":"fact three","role":"fact","action":"CREATE","reason":""},
        {"content":"fact four","role":"fact","action":"CREATE","reason":""}]}"#;
    let (runner, _provider) =
        build_runner(store.storage.clone(), Script::Reply(reply.to_string())).await;
    let job_id = job_row(&store.storage, &run_id).await.2;

    match runner.process_job(&job_id).await {
        JobOutcome::Applied { created, .. } => assert_eq!(created.len(), 3, "spec §6: 最大 3 件"),
        other => panic!("expected an applied extraction, got {other:?}"),
    }
    assert_eq!(memories(&store.storage).await.len(), 3);
}

#[tokio::test]
async fn create_dedupes_and_reinforce_creates_when_nothing_matches() {
    let store = TempStore::new("dedupe").await;
    let run_id = Uuid::new_v4().to_string();
    seed_run(
        &store.storage,
        &run_id,
        json!({"modelId": "p1:m", "providerId": "p1"}),
    )
    .await;

    let existing = MemoryService::create(
        &store.storage,
        MemoryCreate {
            title: "tuesdays".to_string(),
            content: "I run on Tuesdays".to_string(),
            role: "fact".to_string(),
            scope: "user".to_string(),
            source_kind: "user".to_string(),
            task_id: None,
        },
    )
    .await
    .expect("seed memory");

    assert!(enqueue(&store.storage, &run_id, "u", "a")
        .await
        .expect("enqueue"));
    // The model asked to CREATE something that already exists (differing only
    // in case and spacing), and to REINFORCE something that does not.
    let reply = r#"{"memories":[
        {"content":"  i RUN on tuesdays ","role":"fact","action":"CREATE","reason":""},
        {"content":"I write in a bullet journal","role":"fact","action":"REINFORCE","reason":""}]}"#;
    let (runner, _provider) =
        build_runner(store.storage.clone(), Script::Reply(reply.to_string())).await;
    let job_id = job_row(&store.storage, &run_id).await.2;

    match runner.process_job(&job_id).await {
        JobOutcome::Applied {
            created,
            reinforced,
        } => {
            assert_eq!(
                created.len(),
                1,
                "REINFORCE with no match becomes the create"
            );
            assert_eq!(
                reinforced,
                vec![existing.id.clone()],
                "CREATE with a match reinforces"
            );
        }
        other => panic!("expected an applied extraction, got {other:?}"),
    }
    let stored = memories(&store.storage).await;
    assert_eq!(stored.len(), 2, "no duplicate row: {stored:?}");

    let activation: i64 = sqlx::query_scalar("SELECT activation_count FROM memories WHERE id = ?")
        .bind(&existing.id)
        .fetch_one(store.storage.pool())
        .await
        .expect("activation count");
    assert_eq!(
        activation, 1,
        "the identical memory was reinforced exactly once"
    );
}

// ---------------------------------------------------------------------------
// Filtering (spec §6 抽出してはいけないもの + §8)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unstorable_candidates_are_dropped_on_the_apply_path() {
    let store = TempStore::new("filters").await;
    let run_id = Uuid::new_v4().to_string();
    seed_run(
        &store.storage,
        &run_id,
        json!({"modelId": "p1:m", "providerId": "p1"}),
    )
    .await;

    let items = vec![
        ExtractedMemory {
            content: "あ".repeat(201),
            role: "fact".to_string(),
            action: "CREATE".to_string(),
            reason: String::new(),
        },
        ExtractedMemory {
            content: "password: hunter2".to_string(),
            role: "fact".to_string(),
            action: "CREATE".to_string(),
            reason: String::new(),
        },
        ExtractedMemory {
            content: "a role outside the vocabulary".to_string(),
            role: "banana".to_string(),
            action: "CREATE".to_string(),
            reason: String::new(),
        },
        ExtractedMemory {
            content: "the model said no".to_string(),
            role: "fact".to_string(),
            action: "NO_OP".to_string(),
            reason: String::new(),
        },
        ExtractedMemory {
            content: "I keep a paper notebook".to_string(),
            role: "fact".to_string(),
            action: "CREATE".to_string(),
            reason: "durable".to_string(),
        },
    ];
    let applied = apply_extraction(&store.storage, &items, &run_id)
        .await
        .expect("apply");
    assert_eq!(
        applied.created.len(),
        1,
        "only the durable candidate is written"
    );
    assert_eq!(applied.reinforced.len(), 0);

    let stored = memories(&store.storage).await;
    assert_eq!(stored, vec!["I keep a paper notebook".to_string()]);
}

// ---------------------------------------------------------------------------
// Failures: 3 attempts with exponential backoff, then terminal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn three_failed_attempts_end_in_failed() {
    let store = TempStore::new("retries").await;
    let run_id = Uuid::new_v4().to_string();
    seed_run(
        &store.storage,
        &run_id,
        json!({"modelId": "p1:m", "providerId": "p1"}),
    )
    .await;
    assert!(enqueue(&store.storage, &run_id, "u", "a")
        .await
        .expect("enqueue"));

    let (runner, provider) = build_runner(
        store.storage.clone(),
        Script::Error("upstream exploded".to_string()),
    )
    .await;
    let job_id = job_row(&store.storage, &run_id).await.2;

    for attempt in 1..=MAX_ATTEMPTS {
        make_due(&store.storage, &run_id).await;
        match runner.process_job(&job_id).await {
            JobOutcome::RetryScheduled { attempts, error } => {
                assert_eq!(attempts, attempt);
                assert!(
                    error.contains("upstream exploded"),
                    "the provider error is kept: {error}"
                );
            }
            JobOutcome::Failed { attempts, error } => {
                assert_eq!(
                    attempts, MAX_ATTEMPTS,
                    "only the third attempt may fail terminally"
                );
                assert_eq!(attempt, MAX_ATTEMPTS);
                assert!(
                    error.contains("upstream exploded"),
                    "the provider error is kept: {error}"
                );
            }
            other => panic!("attempt {attempt}: expected a failure outcome, got {other:?}"),
        }
    }
    assert_eq!(provider.calls(), 3, "exactly MAX_ATTEMPTS provider calls");

    let (status, attempts, _) = job_row(&store.storage, &run_id).await;
    assert_eq!(status, "failed");
    assert_eq!(attempts, MAX_ATTEMPTS);
    let data = job_data(&store.storage, &run_id).await;
    assert!(data["error"]
        .as_str()
        .unwrap_or_default()
        .contains("upstream exploded"));

    // Terminal: a failed job is never claimed again.
    assert_eq!(runner.process_job(&job_id).await, JobOutcome::Gone);
    assert_eq!(provider.calls(), 3);
}

#[tokio::test]
async fn a_hanging_provider_times_out_into_the_retry_path() {
    let store = TempStore::new("timeout").await;
    let run_id = Uuid::new_v4().to_string();
    seed_run(
        &store.storage,
        &run_id,
        json!({"modelId": "p1:m", "providerId": "p1"}),
    )
    .await;
    assert!(enqueue(&store.storage, &run_id, "u", "a")
        .await
        .expect("enqueue"));

    let (runner, provider) = build_runner(store.storage.clone(), Script::Hang).await;
    let runner = runner.with_timeout(Duration::from_millis(100));
    let job_id = job_row(&store.storage, &run_id).await.2;

    match runner.process_job(&job_id).await {
        JobOutcome::RetryScheduled { attempts, error } => {
            assert_eq!(attempts, 1);
            assert!(
                error.contains("timed out"),
                "the timeout is reported: {error}"
            );
        }
        other => panic!("a hanging provider must take the retry path, got {other:?}"),
    }
    assert_eq!(provider.calls(), 1);

    // The backoff window opens ~30 s out (spec §6 指数的バックオフ).
    let data = job_data(&store.storage, &run_id).await;
    let next = chrono::DateTime::parse_from_rfc3339(
        data["nextAttemptAt"].as_str().expect("next attempt time"),
    )
    .expect("rfc3339")
    .with_timezone(&Utc);
    assert!(
        next > Utc::now() + ChronoDuration::seconds(25),
        "the first retry waits about 30 s: {next}"
    );
    assert_eq!(
        runner.process_job(&job_id).await,
        JobOutcome::NotDue,
        "the closed window keeps the job claimable later, not now"
    );
}

// ---------------------------------------------------------------------------
// The `memory_auto_formation` setting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auto_formation_is_on_by_default() {
    let store = TempStore::new("setting").await;
    assert!(
        auto_formation_enabled(store.storage.pool()).await,
        "no row yet: on"
    );

    sqlx::query("INSERT INTO app_settings (id, owner_id, data) VALUES ('singleton', ?, ?)")
        .bind(SINGLE_USER_ID)
        .bind(r#"{"theme":"dark"}"#)
        .execute(store.storage.pool())
        .await
        .expect("insert settings without the key");
    assert!(
        auto_formation_enabled(store.storage.pool()).await,
        "a row that predates the key reads back as on"
    );

    set_auto_formation(&store.storage, false).await;
    assert!(!auto_formation_enabled(store.storage.pool()).await);
    set_auto_formation(&store.storage, true).await;
    assert!(auto_formation_enabled(store.storage.pool()).await);
}

#[tokio::test]
async fn a_disabled_setting_skips_without_spending_an_attempt() {
    let store = TempStore::new("disabled").await;
    let run_id = Uuid::new_v4().to_string();
    seed_run(
        &store.storage,
        &run_id,
        json!({"modelId": "p1:m", "providerId": "p1"}),
    )
    .await;
    assert!(enqueue(&store.storage, &run_id, "u", "a")
        .await
        .expect("enqueue"));
    set_auto_formation(&store.storage, false).await;

    let (runner, provider) = build_runner(
        store.storage.clone(),
        Script::Reply(r#"{"memories":[]}"#.to_string()),
    )
    .await;
    let job_id = job_row(&store.storage, &run_id).await.2;

    assert_eq!(runner.process_job(&job_id).await, JobOutcome::Skipped);
    let (status, attempts, _) = job_row(&store.storage, &run_id).await;
    assert_eq!(status, "skipped");
    assert_eq!(attempts, 0, "a disabled setting never spends an attempt");
    assert_eq!(provider.calls(), 0, "and never calls the provider");
    assert!(memories(&store.storage).await.is_empty());
}

// ---------------------------------------------------------------------------
// Explicit「覚えて／忘れて」intake (spec §6 明示的な記憶)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn explicit_save_forget_and_secret_refusal() {
    let store = TempStore::new("explicit").await;

    let saved = handle_explicit(&store.storage, "覚えて：私はコーヒーが好き")
        .await
        .expect("explicit save");
    let id = saved.created.expect("a new memory was written");
    assert!(saved.reinforced.is_none());

    // Saying the same thing twice must not create a second row.
    let again = handle_explicit(&store.storage, "覚えて：私はコーヒーが好き")
        .await
        .expect("second explicit save");
    assert_eq!(again.reinforced.as_deref(), Some(id.as_str()));
    assert_eq!(memories(&store.storage).await.len(), 1);

    // 「〜を忘れて」deletes it synchronously, without waiting for extraction.
    let forgotten = handle_explicit(&store.storage, "私はコーヒーが好きを忘れて")
        .await
        .expect("explicit forget");
    assert_eq!(forgotten.forgotten, vec![id]);
    assert!(
        memories(&store.storage).await.is_empty(),
        "the forgotten memory is gone"
    );

    // §8: an explicit request carrying a credential is refused, not stored.
    assert!(
        explicit_candidate("覚えて：password: hunter2").is_none(),
        "the SENSITIVE pattern applies to the candidate too"
    );
    let refused = handle_explicit(&store.storage, "覚えて：password: hunter2")
        .await
        .expect("refusal is not an error");
    assert_eq!(refused, ExplicitOutcome::default());
    assert!(memories(&store.storage).await.is_empty());
}

// ---------------------------------------------------------------------------
// Provider resolution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_provider_falls_back_to_the_model_id_prefix() {
    let store = TempStore::new("provider").await;
    let run_id = Uuid::new_v4().to_string();
    // No `providerId`: the row predates Phase 2, so the prefix decides.
    seed_run(&store.storage, &run_id, json!({"modelId": "p1:fake-model"})).await;
    assert!(enqueue(&store.storage, &run_id, "u", "a")
        .await
        .expect("enqueue"));

    let (runner, provider) = build_runner(
        store.storage.clone(),
        Script::Reply(r#"{"memories":[]}"#.to_string()),
    )
    .await;
    let job_id = job_row(&store.storage, &run_id).await.2;

    match runner.process_job(&job_id).await {
        JobOutcome::Applied {
            created,
            reinforced,
        } => {
            assert!(created.is_empty());
            assert!(reinforced.is_empty());
        }
        other => panic!("the fallback must still reach provider p1, got {other:?}"),
    }
    assert_eq!(provider.calls(), 1);
    assert_eq!(provider.last_request().model, "fake-model");
}

// ---------------------------------------------------------------------------
// The engine side: one job per finished non-temporary run
// ---------------------------------------------------------------------------

async fn run_request(
    engine: &RunEngine,
    workspace: &std::path::Path,
    run_id: Uuid,
    conversation_id: Uuid,
    temporary: bool,
) -> RunRequest {
    RunRequest {
        run_id,
        conversation_id,
        workspace_id: Uuid::new_v4(),
        root_path: Some(workspace.to_string_lossy().to_string()),
        additional_paths: Vec::new(),
        mode: Mode::Thinking,
        model_id: "p1:fake-model".to_string(),
        provider_id: "p1".to_string(),
        system: None,
        verify_command: None,
        format_command: None,
        context_window: 0,
        temperature: None,
        reasoning: None,
        max_tokens: None,
        temporary,
        tools: Arc::new(engine.tool_specs().await),
        history: Vec::new(),
        user_message: "remember this turn".to_string(),
        resume_from: None,
    }
}

async fn wait_for_jobs(storage: &Storage, run_id: &str, expected: i64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let found: i64 = count_for(
            storage,
            "SELECT count(*) FROM memory_processing_jobs WHERE run_id = ?",
            run_id,
        )
        .await;
        if found >= expected {
            return true;
        }
        if tokio::time::Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_finished_run_queues_a_job_and_a_temporary_run_does_not() {
    let store = TempStore::new("engine").await;
    let workspace = store.dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");

    let engine = Arc::new(RunEngine::with_storage_and_tools(
        store.storage.clone(),
        Arc::new(SecretStore::new()),
        ToolRegistry::new(EgressPolicy::default()),
    ));
    let provider = Arc::new(FakeProvider::new(Script::Reply(
        r#"{"memories":[]}"#.to_string(),
    )));
    engine
        .providers
        .register("p1".to_string(), provider.clone())
        .await;
    let mut events = engine.subscribe();
    let _drain = tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            let _ = event;
        }
    });

    // The app inserts the `runs` row before dispatching; extraction needs it.
    let conversation_id = insert_conversation(&store.storage).await;
    let run_id = Uuid::new_v4();
    insert_run(
        &store.storage,
        &run_id.to_string(),
        conversation_id,
        json!({"modelId": "p1:fake-model", "providerId": "p1"}),
    )
    .await;
    let request = run_request(&engine, &workspace, run_id, conversation_id, false).await;
    let outcome = engine.execute(request).await.expect("run");
    assert_eq!(outcome.status, RunStatus::Completed);

    assert!(
        wait_for_jobs(&store.storage, &run_id.to_string(), 1).await,
        "a finished run must queue exactly one extraction job"
    );
    let jobs: i64 = count_for(
        &store.storage,
        "SELECT count(*) FROM memory_processing_jobs WHERE run_id = ?",
        &run_id.to_string(),
    )
    .await;
    assert_eq!(jobs, 1, "one user turn, one extraction");

    // A temporary turn leaves no trace — so it must leave no memory either.
    let temp_conversation_id = insert_conversation(&store.storage).await;
    let temp_run_id = Uuid::new_v4();
    insert_run(
        &store.storage,
        &temp_run_id.to_string(),
        temp_conversation_id,
        json!({"modelId": "p1:fake-model", "providerId": "p1"}),
    )
    .await;
    let request = run_request(&engine, &workspace, temp_run_id, temp_conversation_id, true).await;
    let outcome = engine.execute(request).await.expect("temporary run");
    assert_eq!(outcome.status, RunStatus::Completed);

    tokio::time::sleep(Duration::from_millis(500)).await;
    let jobs: i64 = count_for(
        &store.storage,
        "SELECT count(*) FROM memory_processing_jobs WHERE run_id = ?",
        &temp_run_id.to_string(),
    )
    .await;
    assert_eq!(jobs, 0, "a temporary turn must not queue an extraction");
}
