//! Coding-agent evaluation harness (coding-agent-plan Phase 10.1).
//!
//! The engine tests in `engine_loop.rs` assert *one* behaviour per test. This
//! harness asserts a *task outcome*: a small repository is installed in a temp
//! directory, a scripted Provider drives a realistic trajectory through the
//! real engine and the real tools, and the result is scored the way a user
//! would score it — did the project's own check pass, did the requested change
//! land, and how many turns/tokens did that take?
//!
//! Two modes share the fixtures:
//!
//! * **scripted** (default, runs in CI). Deterministic Provider scripts cover a
//!   single-file fix, a failed-check repair loop, a multi-file rename, and two
//!   guardrails (the verification gate must block a false "done", plan mode
//!   must refuse an edit). This is a regression net for the harness itself:
//!   if an engine change makes a straight-line task need ten extra turns, the
//!   report shows it instead of silently getting slower.
//! * **live** (`--ignored`, requires `MIX_EVAL_*`). The same fixtures are given
//!   to a real OpenAI-compatible model with an empty transcript, and the score
//!   is the independent verifier's verdict. Use it to compare models or to
//!   check whether a prompt/tool change actually helped.
//!
//! Run:
//! ```text
//! cargo test -p mixd-core --test coding_eval -- --nocapture
//! MIX_EVAL_BASE_URL=https://api.example.com/v1 \
//! MIX_EVAL_API_KEY=... MIX_EVAL_MODEL=... \
//!   cargo test -p mixd-core --test coding_eval -- --ignored --nocapture
//! ```
//!
//! The harness never treats model prose as evidence. `false_claim` is recorded
//! when the final answer says the work is done while the independent verifier
//! still fails, which is the failure mode strict verification exists to catch.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mixd_core::mode::Mode;
use mixd_core::runs::engine::{RunEngine, RunRequest};
use mixd_core::runs::state::RunStatus;
use mixd_providers::openai::OpenAiProvider;
use mixd_providers::{
    Provider, ProviderCapabilities, ProviderRequest, StreamBox, StreamEvent, Usage,
};
use mixd_storage::SecretStore;
use mixd_tools::{EgressPolicy, ToolRegistry};
use uuid::Uuid;

const PROVIDER_ID: &str = "eval";

static NEXT_HARNESS: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Scripted Provider
// ---------------------------------------------------------------------------

/// Replays a fixed list of turns, one per Provider call.
///
/// The last turn repeats when the script runs out, which is what makes a
/// "keeps claiming success" scenario terminate through the engine's own repair
/// bound rather than through a panic in the fixture.
struct ScriptedProvider {
    turns: std::sync::Mutex<Vec<Vec<StreamEvent>>>,
    calls: AtomicUsize,
    seen: Arc<std::sync::Mutex<Vec<ProviderRequest>>>,
    name: String,
}

impl ScriptedProvider {
    fn new(turns: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            turns: std::sync::Mutex::new(turns),
            calls: AtomicUsize::new(0),
            seen: Arc::new(std::sync::Mutex::new(Vec::new())),
            name: PROVIDER_ID.to_string(),
        }
    }

    fn requests(&self) -> Vec<ProviderRequest> {
        self.seen.lock().unwrap().clone()
    }

    fn prompt_chars(&self) -> u64 {
        prompt_chars(&self.requests())
    }
}

/// Provider-side prompt size estimate for the whole run.
///
/// A Provider is re-sent the full history every turn, so the same 2 KB tool
/// result costs 2 KB on every later request. Summing the requests (rather than
/// measuring the final context) is what makes a compaction change visible in
/// the score. Four characters per token is the same heuristic the context
/// module uses; this is a cost proxy, not billing.
fn prompt_chars(requests: &[ProviderRequest]) -> u64 {
    requests
        .iter()
        .map(|request| {
            let system = request.system.as_deref().unwrap_or("").chars().count();
            let messages: usize = request
                .messages
                .iter()
                .map(|message| {
                    message.content.chars().count()
                        + message
                            .tool_calls
                            .iter()
                            .map(|call| {
                                call.function.name.chars().count()
                                    + call.function.arguments.chars().count()
                            })
                            .sum::<usize>()
                })
                .sum();
            (system + messages) as u64
        })
        .sum()
}

/// Wraps a real Provider so the harness can measure prompt growth on live runs
/// without teaching every adapter a metrics hook.
struct RecordingProvider {
    inner: Arc<dyn Provider>,
    seen: Arc<std::sync::Mutex<Vec<ProviderRequest>>>,
}

impl RecordingProvider {
    fn new(inner: Arc<dyn Provider>) -> Self {
        Self {
            inner,
            seen: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    fn prompt_chars(&self) -> u64 {
        prompt_chars(&self.seen.lock().unwrap())
    }
}

#[async_trait]
impl Provider for RecordingProvider {
    fn id(&self) -> &String {
        self.inner.id()
    }
    fn label(&self) -> &str {
        self.inner.label()
    }
    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
    async fn stream(&self, req: ProviderRequest) -> anyhow::Result<StreamBox> {
        self.seen.lock().unwrap().push(req.clone());
        self.inner.stream(req).await
    }
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

fn text_turn(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta(text.to_string()),
        StreamEvent::Finished {
            finish_reason: "stop".to_string(),
            usage: Some(Usage {
                input_tokens: 100,
                output_tokens: 20,
                ..Default::default()
            }),
        },
    ]
}

fn tool_turn(id: &str, name: &str, args: &str) -> Vec<StreamEvent> {
    let split = args.len() / 2;
    vec![
        StreamEvent::ToolCallStart {
            id: id.to_string(),
            name: name.to_string(),
        },
        StreamEvent::ToolCallArgumentDelta {
            id: id.to_string(),
            delta: args[..split].to_string(),
        },
        StreamEvent::ToolCallArgumentDelta {
            id: id.to_string(),
            delta: args[split..].to_string(),
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

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    engine: Arc<RunEngine>,
    workspace: PathBuf,
    base: PathBuf,
    storage: Arc<mixd_storage::Storage>,
    conversation_id: Uuid,
}

impl Harness {
    async fn scripted(name: &str, turns: Vec<Vec<StreamEvent>>) -> (Self, Arc<ScriptedProvider>) {
        let provider = Arc::new(ScriptedProvider::new(turns));
        let harness = Self::with_provider(name, provider.clone(), PROVIDER_ID).await;
        (harness, provider)
    }

    async fn with_provider(name: &str, provider: Arc<dyn Provider>, provider_id: &str) -> Self {
        let unique = NEXT_HARNESS.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!(
            "mixd-coding-eval-{}-{unique}-{name}",
            std::process::id()
        ));
        let data_dir = base.join("data");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let storage = Arc::new(
            mixd_storage::Storage::open(&data_dir)
                .await
                .expect("open storage"),
        );
        let engine = Arc::new(RunEngine::with_storage_and_tools(
            storage.clone(),
            Arc::new(SecretStore::new()),
            ToolRegistry::new(EgressPolicy::default()),
        ));
        engine
            .providers
            .register(provider_id.to_string(), provider)
            .await;

        // `messages.conversation_id` references `conversations(id)`.
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
            workspace,
            base,
            storage,
            conversation_id,
        }
    }

    fn install(&self, files: &[(&str, &str)]) {
        for (path, content) in files {
            let target = self.workspace.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(target, content).unwrap();
        }
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.workspace.join(path)).unwrap_or_default()
    }

    async fn allow_all(&self) {
        for tool in [
            "edit",
            "files",
            "terminal",
            "grep",
            "glob",
            "symbols",
            "git",
            "todo",
            "workspace_check",
            "user_plan",
        ] {
            sqlx::query(
                "INSERT OR REPLACE INTO permission_rules (id, owner_id, data) VALUES (?, ?, ?)",
            )
            .bind(format!("eval-allow-{tool}"))
            .bind(mixd_storage::records::SINGLE_USER_ID)
            .bind(
                serde_json::json!({
                    "toolId": tool,
                    "scope": "always",
                    "decision": "allow"
                })
                .to_string(),
            )
            .execute(self.storage.pool())
            .await
            .expect("insert permission rule");
        }
    }

    async fn request(&self, task: &Task, mode: Mode) -> RunRequest {
        RunRequest {
            run_id: Uuid::new_v4(),
            conversation_id: self.conversation_id,
            workspace_id: Uuid::new_v4(),
            root_path: Some(self.workspace.to_string_lossy().to_string()),
            additional_paths: Vec::new(),
            mode,
            model_id: "eval-model".to_string(),
            provider_id: PROVIDER_ID.to_string(),
            system: None,
            verify_command: task.verify.map(str::to_string),
            format_command: None,
            context_window: 0,
            temperature: None,
            reasoning: None,
            max_tokens: None,
            keep_alive: None,
            temporary: false,
            tools: std::sync::Arc::new(self.engine.tool_specs().await),
            history: Vec::new(),
            user_message: task.prompt.to_string(),
            resume_from: None,
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Task {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
    verify: Option<&'static str>,
    prompt: &'static str,
    /// Substrings that must appear in the file after a successful run.
    expected: &'static [(&'static str, &'static str)],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Guardrail {
    None,
    /// The engine must refuse to conclude while the check is failing.
    VerificationGate,
    /// Plan mode must leave the workspace untouched.
    PlanReadOnly,
}

struct Scenario {
    task: Task,
    mode: Mode,
    script: Vec<Vec<StreamEvent>>,
    guardrail: Guardrail,
}

const CALC_PY: &str = "def add(a, b):\n    return a - b\n";
const TEST_CALC_PY: &str = "\
import unittest
from calc import add


class CalcTest(unittest.TestCase):
    def test_add(self):
        self.assertEqual(add(2, 3), 5)


if __name__ == \"__main__\":
    unittest.main()
";

const STATS_PY: &str = "def average(values):\n    return sum(values) / len(values)\n";
const TEST_STATS_PY: &str = "\
import unittest
from stats import average


class StatsTest(unittest.TestCase):
    def test_average(self):
        self.assertEqual(average([2, 4]), 3)

    def test_empty(self):
        self.assertEqual(average([]), 0)


if __name__ == \"__main__\":
    unittest.main()
";

const LIB_PY: &str = "def add(left, right):\n    return left + right\n";
const APP_PY: &str = "\
from lib import add


def total():
    return add(2, 3)
";
const TEST_APP_PY: &str = "\
import unittest
from app import total
from lib import plus


class AppTest(unittest.TestCase):
    def test_total(self):
        self.assertEqual(total(), 5)

    def test_plus(self):
        self.assertEqual(plus(1, 1), 2)


if __name__ == \"__main__\":
    unittest.main()
";

fn arg(value: serde_json::Value) -> String {
    value.to_string()
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            task: Task {
                name: "single_file_bugfix",
                files: &[("calc.py", CALC_PY), ("test_calc.py", TEST_CALC_PY)],
                verify: Some("python3 -B -m unittest -q"),
                prompt: "The test suite fails. Find the bug and fix it. Run the tests to \
                         confirm they pass.",
                expected: &[("calc.py", "return a + b")],
            },
            mode: Mode::Agent,
            script: vec![
                tool_turn(
                    "r1",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "calc.py"})),
                ),
                tool_turn(
                    "e1",
                    "edit",
                    &arg(serde_json::json!({
                        "path": "calc.py",
                        "old_string": "return a - b",
                        "new_string": "return a + b"
                    })),
                ),
                tool_turn(
                    "r2",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "calc.py"})),
                ),
                text_turn("Fixed `add` in `calc.py`; `python3 -B -m unittest -q` passes."),
            ],
            guardrail: Guardrail::None,
        },
        Scenario {
            task: Task {
                name: "test_repair_after_failed_check",
                files: &[("stats.py", STATS_PY), ("test_stats.py", TEST_STATS_PY)],
                verify: Some("python3 -B -m unittest -q"),
                prompt: "The test suite fails. Fix the implementation so every test passes, \
                         and run the tests to confirm.",
                expected: &[("stats.py", "if values else 0")],
            },
            mode: Mode::Agent,
            script: vec![
                tool_turn(
                    "r1",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "stats.py"})),
                ),
                // A plausible but wrong first attempt: empty input returns 1.
                tool_turn(
                    "e1",
                    "edit",
                    &arg(serde_json::json!({
                        "path": "stats.py",
                        "old_string": "return sum(values) / len(values)",
                        "new_string": "return sum(values) / len(values) if values else 1"
                    })),
                ),
                // Claims success; the engine has to send it back to work.
                text_turn("Done — all tests pass now."),
                // The first edit changed the file, so the anchor for the second
                // one has to come from a fresh read.
                tool_turn(
                    "r2",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "stats.py"})),
                ),
                tool_turn(
                    "e2",
                    "edit",
                    &arg(serde_json::json!({
                        "path": "stats.py",
                        "old_string": "return sum(values) / len(values) if values else 1",
                        "new_string": "return sum(values) / len(values) if values else 0"
                    })),
                ),
                tool_turn(
                    "r3",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "stats.py"})),
                ),
                tool_turn(
                    "t1",
                    "terminal",
                    &arg(serde_json::json!({"command": "python3 -B -m unittest -q"})),
                ),
                text_turn("Fixed `average([])`; `python3 -B -m unittest -q` passes."),
            ],
            guardrail: Guardrail::None,
        },
        Scenario {
            task: Task {
                name: "multi_file_rename",
                files: &[
                    ("lib.py", LIB_PY),
                    ("app.py", APP_PY),
                    ("test_app.py", TEST_APP_PY),
                ],
                verify: Some("python3 -B -m unittest -q"),
                prompt: "Rename `add` to `plus` in `lib.py`, update every caller, and keep the \
                         tests passing. Run the tests to confirm.",
                expected: &[("lib.py", "def plus("), ("app.py", "from lib import plus")],
            },
            mode: Mode::Agent,
            script: vec![
                tool_turn(
                    "r1",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "lib.py"})),
                ),
                tool_turn(
                    "r2",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "app.py"})),
                ),
                tool_turn(
                    "e1",
                    "edit",
                    &arg(serde_json::json!({
                        "edits": [
                            {"path": "lib.py", "old_string": "def add(", "new_string": "def plus("},
                            {
                                "path": "app.py",
                                "old_string": "from lib import add\n\n\ndef total():\n    return add(2, 3)",
                                "new_string": "from lib import plus\n\n\ndef total():\n    return plus(2, 3)"
                            }
                        ]
                    })),
                ),
                tool_turn(
                    "r3",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "lib.py"})),
                ),
                tool_turn(
                    "r4",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "app.py"})),
                ),
                text_turn("Renamed `add` to `plus` in both files; the tests pass."),
            ],
            guardrail: Guardrail::None,
        },
        Scenario {
            task: Task {
                name: "verification_gate_blocks_false_claim",
                files: &[("calc.py", CALC_PY), ("test_calc.py", TEST_CALC_PY)],
                verify: Some("python3 -B -m unittest -q"),
                prompt: "Make the tests pass.",
                expected: &[],
            },
            mode: Mode::Agent,
            script: vec![
                tool_turn(
                    "r1",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "calc.py"})),
                ),
                tool_turn(
                    "e1",
                    "edit",
                    &arg(serde_json::json!({
                        "path": "calc.py",
                        "old_string": "return a - b",
                        "new_string": "return a * b"
                    })),
                ),
                // The last turn repeats: the run must end because the engine
                // gives up on verification, not because the script ran out.
                text_turn("Done — the tests pass."),
            ],
            guardrail: Guardrail::VerificationGate,
        },
        Scenario {
            task: Task {
                name: "plan_mode_refuses_edit",
                files: &[("calc.py", CALC_PY)],
                verify: None,
                prompt: "Plan how to fix the tests. Do not change anything.",
                expected: &[("calc.py", "return a - b")],
            },
            mode: Mode::Plan,
            script: vec![
                tool_turn(
                    "r1",
                    "files",
                    &arg(serde_json::json!({"action": "read", "path": "calc.py"})),
                ),
                tool_turn(
                    "e1",
                    "edit",
                    &arg(serde_json::json!({
                        "path": "calc.py",
                        "old_string": "return a - b",
                        "new_string": "return a + b"
                    })),
                ),
                text_turn("Plan: change `return a - b` to `return a + b`."),
            ],
            guardrail: Guardrail::PlanReadOnly,
        },
    ]
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

struct RunScore {
    task: &'static str,
    passed: bool,
    steps: u32,
    tool_calls: usize,
    prompt_tokens: u64,
    wall_ms: u128,
    status: RunStatus,
    false_claim: bool,
    detail: String,
}

fn claims_success(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "tests pass",
        "test passes",
        "all tests",
        "fixed",
        "done",
        "completed",
        "成功",
        "完了",
        "通りました",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// Run the project's own check outside the agent, so a passing answer cannot be
/// self-certified. Returns `(ok, output)`.
fn independent_verify(task: &Task, workspace: &Path) -> (bool, String) {
    let Some(command) = task.verify else {
        return (true, String::new());
    };
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(workspace)
        .output()
        .expect("run verify command");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

fn expected_present(task: &Task, harness: &Harness) -> bool {
    task.expected
        .iter()
        .all(|(path, needle)| harness.read(path).contains(needle))
}

async fn run_scripted(scenario: &Scenario) -> RunScore {
    let (harness, provider) = Harness::scripted(scenario.task.name, scenario.script.clone()).await;
    harness.install(scenario.task.files);
    harness.allow_all().await;

    let mut events = harness.engine.subscribe();
    let started = Instant::now();
    let outcome = harness
        .engine
        .execute(harness.request(&scenario.task, scenario.mode).await)
        .await
        .expect("run");
    let wall_ms = started.elapsed().as_millis();

    let mut kinds = Vec::new();
    while let Ok(event) = events.try_recv() {
        kinds.push(event.kind);
    }

    let (verified, output) = independent_verify(&scenario.task, &harness.workspace);
    let expected = expected_present(&scenario.task, &harness);
    let tool_errors: Vec<String> = outcome
        .tool_calls
        .iter()
        .filter_map(|call| call.error.clone())
        .collect();

    let (passed, detail) = match scenario.guardrail {
        Guardrail::None => (
            verified && expected,
            if verified && expected {
                "task completed and verified".to_string()
            } else {
                format!("verified={verified} expected={expected} errors={tool_errors:?}: {output}")
            },
        ),
        Guardrail::VerificationGate => {
            let gated = kinds.iter().any(|kind| kind == "verification_failed");
            (
                gated && !verified,
                if gated {
                    "engine rejected the unverified conclusion".to_string()
                } else {
                    "engine let an unverified conclusion through".to_string()
                },
            )
        }
        Guardrail::PlanReadOnly => {
            // A refused edit must not arm the verification gate: nothing was
            // written, so demanding a confirming read would be a false alarm
            // that ends the Run in `verification_failed`.
            let gated = kinds
                .iter()
                .any(|kind| kind == "verification_required" || kind == "verification_failed");
            (
                expected && !gated,
                if !expected {
                    "plan mode wrote to the workspace".to_string()
                } else if gated {
                    "a refused edit still armed the verification gate".to_string()
                } else {
                    "plan mode left the workspace untouched".to_string()
                },
            )
        }
    };

    if std::env::var("MIX_EVAL_DEBUG").is_ok() {
        eprintln!(
            "[debug] {} steps={} calls={:?} kinds={:?}",
            scenario.task.name,
            outcome.steps,
            outcome
                .tool_calls
                .iter()
                .map(|call| (call.name.as_str(), call.error.as_deref()))
                .collect::<Vec<_>>(),
            kinds,
        );
        for call in &outcome.tool_calls {
            if call.name == "terminal" {
                eprintln!("[debug] terminal result={:?}", call.result);
            }
        }
    }

    RunScore {
        task: scenario.task.name,
        passed,
        steps: outcome.steps,
        tool_calls: outcome.tool_calls.len(),
        prompt_tokens: provider.prompt_chars() / 4,
        wall_ms,
        status: outcome.status,
        false_claim: claims_success(&outcome.text) && !verified,
        detail,
    }
}

fn print_report(scores: &[RunScore], title: &str) {
    println!("\n{title}");
    println!(
        "{:<36} {:>6} {:>5} {:>6} {:>10} {:>9} {:>6}",
        "task", "result", "steps", "tools", "prompt_tok", "wall_ms", "claim"
    );
    for score in scores {
        println!(
            "{:<36} {:>6} {:>5} {:>6} {:>10} {:>9} {:>6}",
            score.task,
            if score.passed { "PASS" } else { "FAIL" },
            score.steps,
            score.tool_calls,
            score.prompt_tokens,
            score.wall_ms,
            if score.false_claim { "FALSE" } else { "-" },
        );
        if !score.passed {
            println!("  {} — {}", score.detail, score.status.as_str());
        }
    }

    let tasks: Vec<&RunScore> = scores
        .iter()
        .filter(|score| {
            score.task != "verification_gate_blocks_false_claim"
                && score.task != "plan_mode_refuses_edit"
        })
        .collect();
    let guardrails: Vec<&RunScore> = scores
        .iter()
        .filter(|score| !tasks.iter().any(|t| t.task == score.task))
        .collect();
    let sum =
        |items: &[&RunScore], pass: bool| items.iter().filter(|score| score.passed == pass).count();
    let total_prompt: u64 = scores.iter().map(|score| score.prompt_tokens).sum();
    let avg_steps = scores.iter().map(|s| s.steps as f64).sum::<f64>() / scores.len() as f64;
    println!(
        "summary: tasks {}/{}, guardrails {}/{}, avg steps {:.1}, prompt tokens {}",
        sum(&tasks, true),
        tasks.len(),
        sum(&guardrails, true),
        guardrails.len(),
        avg_steps,
        total_prompt
    );
}

#[tokio::test]
async fn scripted_fixtures_are_solved_within_budget() {
    let mut scores = Vec::new();
    for scenario in scenarios() {
        scores.push(run_scripted(&scenario).await);
    }
    print_report(&scores, "coding eval — scripted");

    for score in &scores {
        assert!(score.passed, "{} failed: {}", score.task, score.detail);
        assert!(
            score.steps <= 12,
            "{} used {} steps; a straight-line fixture should not need more than 12",
            score.task,
            score.steps
        );
    }

    let gate = scores
        .iter()
        .find(|score| score.task == "verification_gate_blocks_false_claim")
        .expect("the false-claim scenario must be scored");
    assert!(
        gate.false_claim,
        "the harness must record a success claim while the check still fails"
    );

    let tasks: u64 = scores
        .iter()
        .filter(|s| {
            s.task != "verification_gate_blocks_false_claim" && s.task != "plan_mode_refuses_edit"
        })
        .map(|s| s.prompt_tokens)
        .sum();
    assert!(
        tasks < 200_000,
        "prompt cost regressed: {tasks} estimated tokens across the three coding tasks"
    );
}

// ---------------------------------------------------------------------------
// Live mode
// ---------------------------------------------------------------------------

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[tokio::test]
#[ignore = "requires MIX_EVAL_BASE_URL, MIX_EVAL_API_KEY and MIX_EVAL_MODEL"]
async fn live_model_runs_the_fixture_tasks() {
    let (Some(base_url), Some(model)) = (env("MIX_EVAL_BASE_URL"), env("MIX_EVAL_MODEL")) else {
        eprintln!("MIX_EVAL_BASE_URL and MIX_EVAL_MODEL are required; skipping");
        return;
    };
    let api_key = env("MIX_EVAL_API_KEY");
    let timeout_secs: u64 = env("MIX_EVAL_TIMEOUT_SECS")
        .and_then(|value| value.parse().ok())
        .unwrap_or(180);

    let provider: Arc<dyn Provider> = Arc::new(OpenAiProvider::new(base_url, api_key));

    let mut scores = Vec::new();
    for scenario in scenarios()
        .into_iter()
        .filter(|scenario| scenario.guardrail == Guardrail::None)
    {
        // One recorder per task: the prompt total has to belong to this run,
        // not to every task that ran before it.
        let recorder = Arc::new(RecordingProvider::new(provider.clone()));
        let harness =
            Harness::with_provider(scenario.task.name, recorder.clone(), PROVIDER_ID).await;
        harness.install(scenario.task.files);
        harness.allow_all().await;

        let mut events = harness.engine.subscribe();
        let started = Instant::now();
        let mut request = harness.request(&scenario.task, Mode::Agent).await;
        request.model_id = model.clone();
        request.context_window = env("MIX_EVAL_CONTEXT_WINDOW")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);

        let outcome = match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            harness.engine.execute(request),
        )
        .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(err)) => {
                eprintln!("{}: provider failed: {err}", scenario.task.name);
                continue;
            }
            Err(_) => {
                eprintln!("{}: timed out after {timeout_secs}s", scenario.task.name);
                continue;
            }
        };
        let wall_ms = started.elapsed().as_millis();
        while events.try_recv().is_ok() {}

        let (verified, output) = independent_verify(&scenario.task, &harness.workspace);
        let expected = expected_present(&scenario.task, &harness);
        scores.push(RunScore {
            task: scenario.task.name,
            passed: verified && expected,
            steps: outcome.steps,
            tool_calls: outcome.tool_calls.len(),
            prompt_tokens: recorder.prompt_chars() / 4,
            wall_ms,
            status: outcome.status,
            false_claim: claims_success(&outcome.text) && !verified,
            detail: if verified && expected {
                "task completed and verified".to_string()
            } else {
                format!("verified={verified} expected={expected}: {output}")
            },
        });
    }

    print_report(&scores, &format!("coding eval — live ({model})"));

    if let Some(min) = env("MIX_EVAL_MIN_PASS").and_then(|value| value.parse::<usize>().ok()) {
        let passed = scores.iter().filter(|score| score.passed).count();
        assert!(
            passed >= min,
            "{passed} of {} tasks passed; MIX_EVAL_MIN_PASS={min}",
            scores.len()
        );
    }
}
