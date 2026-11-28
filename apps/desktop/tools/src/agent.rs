//! Delegation to a saved agent profile.
//!
//! An agent profile is a named set of instructions plus the tools it may use.
//! Without this tool a profile was decoration: it could be created and listed in
//! the UI, but nothing in the engine read it, so `toolIds` filtered nothing and
//! `instructions` never reached a model.
//!
//! The delegated agent runs as a fresh sub-run with its own budget and its own
//! conversation, which is what keeps a long exploration from filling the parent's
//! context. The parent gets back the sub-agent's final text and its tool-call
//! trace, so the choice of delegate is visible in the transcript.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::{Tool, ToolContext, ToolResult};

/// A stored agent profile.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub instructions: String,
    /// Permitted tool ids. Empty means "the parent's tools, unchanged".
    #[serde(default)]
    pub tool_ids: Vec<String>,
    /// Optional `provider:model` override. Empty means "the parent's model",
    /// which is the right default: a cheaper model is fine for a read-only
    /// explorer but wrong for a delegate that has to edit code.
    #[serde(default)]
    pub model_id: Option<String>,
}

/// Token spend of one finished sub-run, reported separately from the
/// parent's own budget. The child ran under its own limits; these numbers are
/// aggregation-only and never deduct from the parent.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ChildUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// A finished sub-run, handed back to the parent.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DelegateResult {
    pub agent: String,
    pub text: String,
    pub steps: u32,
    pub tool_calls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Token spend of this sub-run alone. See [`ChildUsage`]: the parent
    /// aggregates these for reporting, it does not share limits with them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChildUsage>,
    /// Temporary worktree the sub-agent ran in, when the implementation keeps
    /// one. The built-in isolated delegate removes its worktree before
    /// returning, so it reports `None` and relies on `diff`/`changed_files`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    /// Unified diff produced in that worktree (truncated for the transcript).
    /// The parent can apply it or review the changed files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_files: Vec<String>,
    /// Whether the `diff` still applies cleanly on the parent workspace
    /// (`git apply --check` at return time). `None` when there is no diff to
    /// judge (read-only delegate, or isolation unavailable). The parent merges
    /// deliberately — this flag is the confirmation input, not an auto-apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mergeable: Option<bool>,
    /// The base commit the isolated worktree was cut from, so the parent can
    /// re-verify or cherry-pick the patch later.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_base: Option<String>,
    /// The child's conclusion parsed as JSON when the caller supplied a
    /// `schema`. `None` when no schema was given or the text was not valid
    /// JSON — `text` always keeps the raw conclusion either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
}

/// Supplies the saved profiles. Implemented by the app shell.
#[async_trait]
pub trait AgentSource: Send + Sync {
    async fn profiles(&self) -> anyhow::Result<Vec<AgentProfile>>;
}

/// Runs a delegated agent. Implemented by the engine, which owns the run loop.
#[async_trait]
pub trait AgentRunner: Send + Sync {
    /// Execute `task` under `profile` and return what it concluded.
    async fn delegate(
        &self,
        parent: &ToolContext,
        profile: &AgentProfile,
        task: &str,
    ) -> anyhow::Result<DelegateResult>;

    /// Run in a throwaway git worktree. The default falls back to `delegate`
    /// so a runner without git support still works; the app runner overrides
    /// it and reports the diff.
    async fn delegate_isolated(
        &self,
        parent: &ToolContext,
        profile: &AgentProfile,
        task: &str,
    ) -> anyhow::Result<DelegateResult> {
        self.delegate(parent, profile, task).await
    }
}

pub struct AgentTool {
    source: Arc<dyn AgentSource>,
    runner: Arc<dyn AgentRunner>,
    cache: Arc<RwLock<Option<Vec<AgentProfile>>>>,
}

impl AgentTool {
    pub fn new(source: Arc<dyn AgentSource>, runner: Arc<dyn AgentRunner>) -> Self {
        Self {
            source,
            runner,
            cache: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn profiles(&self) -> anyhow::Result<Vec<AgentProfile>> {
        if let Some(hit) = self.cache.read().await.as_ref() {
            return Ok(hit.clone());
        }
        let loaded = self.source.profiles().await?;
        *self.cache.write().await = Some(loaded.clone());
        Ok(loaded)
    }

    pub async fn invalidate(&self) {
        *self.cache.write().await = None;
    }

    /// Narrow a tool list to what `profile` permits.
    ///
    /// An empty `tool_ids` means unrestricted, which is how a profile says "same
    /// tools as the caller". A non-empty list is a hard filter: the sub-agent is
    /// never even *shown* a tool it may not use, because a tool the model can
    /// see but not call is a reliable source of retries.
    pub fn permitted(specs: Vec<super::ToolSpec>, allowed: &[String]) -> Vec<super::ToolSpec> {
        if allowed.is_empty() {
            return specs;
        }
        specs
            .into_iter()
            .filter(|spec| {
                allowed
                    .iter()
                    .any(|id| id == &spec.name || id == "*" || glob_matches(id, &spec.name))
            })
            .collect()
    }
}

/// Minimal `prefix*` support, because profiles name families like `mcp__github__*`
/// rather than enumerating every tool a server exposes.
fn glob_matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn id(&self) -> &str {
        "agent"
    }
    fn name(&self) -> &str {
        "Agent"
    }
    fn description(&self) -> &str {
        "Delegate self-contained work to a saved agent profile. The sub-agent gets its \
         own context and budget and returns only its conclusion, so use it for open-ended \
         exploration rather than for steps you could do yourself. Pass `tasks` to fan out \
         to several sub-agents at once. Call with no arguments to list the profiles."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent": { "type": "string", "description": "Profile name. Omit to list them." },
                "task": {
                    "type": "string",
                    "description": "The full task, including everything the sub-agent needs. It cannot see this conversation."
                },
                "tasks": {
                    "type": "array",
                    "items": {
                        "anyOf": [
                            { "type": "string" },
                            {
                                "type": "object",
                                "properties": {
                                    "task": { "type": "string" },
                                    "schema": {
                                        "type": "object",
                                        "description": "JSON Schema for this task's conclusion. Overrides the top-level `schema`."
                                    }
                                },
                                "required": ["task"]
                            }
                        ]
                    },
                    "description": "Several independent tasks to run in parallel (at most 8 at a time). Prefer this over repeated calls when the sub-tasks do not depend on each other. A task may be a plain string or {task, schema}."
                },
                "schema": {
                    "type": "object",
                    "description": "JSON Schema for the sub-agent's conclusion. The task text asks for a single JSON value conforming to it, and a parsed copy is returned as `structured` (raw text stays in `text`). Applies to every task unless that task carries its own `schema`."
                },
                "auto_merge": {
                    "type": "boolean",
                    "description": "When true and `isolate` is on, the response adds `merge_ready` (every diff applies cleanly and no two tasks touched the same file) plus `merge_conflicts` listing overlapping files. Nothing is merged automatically either way; the parent applies the diffs deliberately."
                },
                "isolate": {
                    "type": "boolean",
                    "description": "Run in a throwaway git worktree so parallel edits cannot collide. Defaults to true when `tasks` is used. The parent receives the diff and changed file list; nothing is merged automatically."
                }
            }
        })
    }

    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let profiles = self.profiles().await?;
        if profiles.is_empty() {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({ "agents": [] }),
                error: Some("no agent profiles are defined".to_string()),
            });
        }
        let name = args
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            return Ok(ToolResult {
                ok: true,
                content: serde_json::json!({
                    "agents": profiles
                        .iter()
                        .map(|p| serde_json::json!({
                            "name": p.name,
                            "tools": p.tool_ids,
                        }))
                        .collect::<Vec<_>>(),
                }),
                error: None,
            });
        }
        let Some(profile) = profiles.iter().find(|p| p.name.eq_ignore_ascii_case(name)) else {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({
                    "available": profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>()
                }),
                error: Some(format!("no agent profile named {name:?}")),
            });
        };
        // A task is a plain string or { task, schema }. The top-level
        // `schema` is the default for tasks that do not carry their own.
        // Strings stay valid: older callers never sent objects.
        let default_schema: Option<serde_json::Value> = args.get("schema").cloned();
        let mut tasks: Vec<(String, Option<serde_json::Value>)> = args
            .get("tasks")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| {
                        if let Some(text) = v.as_str() {
                            let text = text.trim();
                            if text.is_empty() {
                                return None;
                            }
                            Some((text.to_string(), default_schema.clone()))
                        } else {
                            let text = v.get("task")?.as_str()?.trim();
                            if text.is_empty() {
                                return None;
                            }
                            let schema =
                                v.get("schema").cloned().or_else(|| default_schema.clone());
                            Some((text.to_string(), schema))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if tasks.is_empty() {
            if let Some(task) = args
                .get("task")
                .and_then(|v| v.as_str())
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
            {
                tasks.push((task.to_string(), default_schema.clone()));
            }
        }
        if tasks.is_empty() {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({ "agent": profile.name }),
                error: Some("task or tasks is required when delegating".to_string()),
            });
        }
        // Concurrency chunking bounds how many sub-runs are *live*, but a
        // caller could still ask for thousands of tasks and keep this Run busy
        // (and the provider bill climbing) for hours. Cap the total too.
        if tasks.len() > MAX_TOTAL_DELEGATES {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({
                    "agent": profile.name,
                    "requested_tasks": tasks.len(),
                    "max_tasks": MAX_TOTAL_DELEGATES,
                }),
                error: Some(format!(
                    "too many tasks: {} requested, at most {MAX_TOTAL_DELEGATES} per delegation; split the work into smaller batches",
                    tasks.len()
                )),
            });
        }

        // Fan-out almost always means independent edits; isolate those by
        // default. A single delegation stays in the parent workspace unless the
        // caller asks otherwise (a worktree round-trip is measurable on a large
        // repository and pointless for a read-only explorer).
        let isolate = args
            .get("isolate")
            .and_then(|value| value.as_bool())
            .unwrap_or(tasks.len() > 1);
        // Confirmation flow for worktree fan-out: report whether the collected
        // diffs are safe to apply, but never apply them here.
        let auto_merge = args
            .get("auto_merge")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        if tasks.len() == 1 {
            let (task, schema) = tasks.remove(0);
            return Ok(self
                .delegate_one(&ctx, profile, task, schema, isolate)
                .await);
        }

        // Fan-out: independent sub-tasks run at the same time. Each one still
        // owns its own context and budget, which is the point — a big
        // exploration split three ways costs one wall-clock wait, not three.
        // Child budgets stay separate; the parent only aggregates the totals
        // below for reporting, it never shares limits with them.
        use futures_util::StreamExt;
        let mut results: Vec<serde_json::Value> = Vec::with_capacity(tasks.len());
        let ctx_ref = &ctx;
        for chunk in tasks.chunks(MAX_PARALLEL_DELEGATES) {
            let mut in_flight = futures_util::stream::FuturesUnordered::new();
            for (task, schema) in chunk {
                in_flight.push(async move {
                    let outcome = self
                        .delegate_one(ctx_ref, profile, task.clone(), schema.clone(), isolate)
                        .await;
                    (task.clone(), outcome)
                });
            }
            while let Some((task, outcome)) = in_flight.next().await {
                let mut value = outcome.content;
                if let Some(map) = value.as_object_mut() {
                    map.insert("task".to_string(), serde_json::json!(task));
                }
                results.push(value);
            }
        }
        let any_failed = results.iter().any(|item| item.get("error").is_some());
        // Separate aggregation of the children's own budgets: steps, tool
        // calls and tokens are summed for the transcript, while the limits
        // that produced them belonged to each sub-run alone.
        let mut steps_total: u64 = 0;
        let mut tool_calls_total: u64 = 0;
        let mut input_tokens_total: u64 = 0;
        let mut output_tokens_total: u64 = 0;
        for item in &results {
            steps_total =
                steps_total.saturating_add(item.get("steps").and_then(|v| v.as_u64()).unwrap_or(0));
            tool_calls_total = tool_calls_total.saturating_add(
                item.get("tool_calls")
                    .and_then(|v| v.as_array())
                    .map(|calls| calls.len() as u64)
                    .unwrap_or(0),
            );
            if let Some(usage) = item.get("usage") {
                input_tokens_total = input_tokens_total.saturating_add(
                    usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                );
                output_tokens_total = output_tokens_total.saturating_add(
                    usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                );
            }
        }
        // Overlapping `changed_files` across isolated worktrees mean the diffs
        // cannot all apply: flag them so the parent merges deliberately.
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for item in &results {
            if let Some(files) = item.get("changed_files").and_then(|v| v.as_array()) {
                for file in files.iter().filter_map(|f| f.as_str()) {
                    *seen.entry(file.to_string()).or_insert(0) += 1;
                }
            }
        }
        let mut merge_conflicts: Vec<String> = seen
            .into_iter()
            .filter(|(_, n)| *n > 1)
            .map(|(f, _)| f)
            .collect();
        merge_conflicts.sort();
        let merge_ready = auto_merge
            && isolate
            && merge_conflicts.is_empty()
            && !results.is_empty()
            && results
                .iter()
                .all(|item| item.get("mergeable") != Some(&serde_json::json!(false)));
        let task_count = results.len();
        Ok(ToolResult {
            ok: !any_failed,
            content: serde_json::json!({
                "agent": profile.name,
                "results": results,
                "child_budget": {
                    "tasks": task_count,
                    "steps_total": steps_total,
                    "tool_calls_total": tool_calls_total,
                    "input_tokens_total": input_tokens_total,
                    "output_tokens_total": output_tokens_total,
                    "total_tokens": input_tokens_total.saturating_add(output_tokens_total),
                },
                "merge_conflicts": merge_conflicts,
                "merge_ready": merge_ready,
            }),
            error: any_failed.then(|| "one or more delegated tasks failed".to_string()),
        })
    }
}

impl AgentTool {
    async fn delegate_one(
        &self,
        ctx: &ToolContext,
        profile: &AgentProfile,
        task: String,
        schema: Option<serde_json::Value>,
        isolate: bool,
    ) -> ToolResult {
        // A schema does not change how the sub-agent runs — it only appends a
        // response contract to the task text, so runners without structured
        // support still work and return plain text.
        let effective = match &schema {
            Some(schema) => format!(
                "{task}\n\nRespond with a single JSON value conforming to this JSON Schema. \
                 Output only the JSON, no prose outside it:\n{schema}"
            ),
            None => task,
        };
        let outcome = if isolate {
            self.runner
                .delegate_isolated(ctx, profile, &effective)
                .await
        } else {
            self.runner.delegate(ctx, profile, &effective).await
        };
        match outcome {
            Ok(mut result) => {
                // Best-effort structured copy: a child that ignored the
                // contract still reports its raw text, never a new failure.
                if schema.is_some() && result.structured.is_none() {
                    result.structured = parse_structured(&result.text);
                }
                let failed = result.error.is_some();
                ToolResult {
                    ok: !failed,
                    content: serde_json::to_value(&result)
                        .unwrap_or_else(|_| serde_json::json!({})),
                    error: result.error,
                }
            }
            Err(err) => ToolResult {
                ok: false,
                content: serde_json::json!({ "agent": profile.name, "error": err.to_string() }),
                error: Some(format!("delegation failed: {err}")),
            },
        }
    }
}

/// Pull a JSON value out of a child's conclusion: raw JSON first, then a
/// single fenced code block. Returns `None` for prose, which simply means the
/// caller reads `text` instead of `structured`.
fn parse_structured(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(value);
    }
    for fence in ["```json", "```"] {
        if let Some(rest) = trimmed.strip_prefix(fence) {
            // The text ends with the closing fence, so the first `rsplit`
            // piece is empty — the payload is the one before it. Without a
            // closing fence at all, try whatever follows the opener.
            let inner = rest
                .rsplit("```")
                .nth(1)
                .or(Some(rest))
                .unwrap_or("")
                .trim();
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(inner) {
                return Some(value);
            }
        }
    }
    None
}

/// Delegates fan out at most this many at a time (workflow-grade fan-out:
/// eight parallel sub-runs). Each sub-agent is a full Run with its own Tool
/// calls, so the fan-out stays bounded even though it is wider than before —
/// unbounded delegation would multiply provider spend and local I/O.
const MAX_PARALLEL_DELEGATES: usize = 8;

/// Hard cap on the number of tasks in a single delegation, including chunks.
/// `MAX_PARALLEL_DELEGATES` bounds concurrency; this bounds total work so a
/// single Tool call cannot fan out into an unbounded number of sub-runs.
const MAX_TOTAL_DELEGATES: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticAgents(Vec<AgentProfile>);

    #[async_trait]
    impl AgentSource for StaticAgents {
        async fn profiles(&self) -> anyhow::Result<Vec<AgentProfile>> {
            Ok(self.0.clone())
        }
    }

    struct EchoRunner;

    #[async_trait]
    impl AgentRunner for EchoRunner {
        async fn delegate(
            &self,
            _parent: &ToolContext,
            profile: &AgentProfile,
            task: &str,
        ) -> anyhow::Result<DelegateResult> {
            Ok(DelegateResult {
                agent: profile.name.clone(),
                text: format!("handled: {task}"),
                steps: 3,
                tool_calls: vec!["grep".into()],
                error: None,
                ..Default::default()
            })
        }
    }

    struct FailingRunner;

    /// Returns its conclusion as raw JSON, for the `schema` path.
    struct JsonRunner;

    #[async_trait]
    impl AgentRunner for JsonRunner {
        async fn delegate(
            &self,
            _parent: &ToolContext,
            profile: &AgentProfile,
            _task: &str,
        ) -> anyhow::Result<DelegateResult> {
            Ok(DelegateResult {
                agent: profile.name.clone(),
                text: r#"{"answer": 42}"#.to_string(),
                steps: 1,
                tool_calls: Vec::new(),
                error: None,
                ..Default::default()
            })
        }
    }

    #[async_trait]
    impl AgentRunner for FailingRunner {
        async fn delegate(
            &self,
            _parent: &ToolContext,
            profile: &AgentProfile,
            _task: &str,
        ) -> anyhow::Result<DelegateResult> {
            Ok(DelegateResult {
                agent: profile.name.clone(),
                text: String::new(),
                steps: 0,
                tool_calls: Vec::new(),
                error: Some(format!("{} ran out of budget", profile.name)),
                ..Default::default()
            })
        }
    }

    fn profiles() -> Vec<AgentProfile> {
        vec![
            AgentProfile {
                id: "1".into(),
                name: "explorer".into(),
                instructions: "Read-only. Never edits.".into(),
                tool_ids: vec!["files".into(), "grep".into(), "glob".into()],
                model_id: None,
            },
            AgentProfile {
                id: "2".into(),
                name: "builder".into(),
                instructions: "Makes changes.".into(),
                tool_ids: vec!["files".into(), "edit".into(), "terminal".into()],
                model_id: None,
            },
        ]
    }

    fn ctx() -> ToolContext {
        ToolContext {
            run_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: None,
            additional_paths: Vec::new(),
            approval_token: None,
        }
    }

    fn spec(name: &str) -> super::super::ToolSpec {
        super::super::ToolSpec {
            name: name.to_string(),
            description: String::new(),
            parameters: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn an_empty_allowlist_means_unrestricted() {
        let all = vec![spec("files"), spec("terminal"), spec("grep")];
        assert_eq!(AgentTool::permitted(all.clone(), &[]).len(), 3);
    }

    #[test]
    fn a_allowlist_filters_hard() {
        let all = vec![spec("files"), spec("terminal"), spec("grep")];
        let kept = AgentTool::permitted(all, &["files".to_string(), "grep".to_string()]);
        let names: Vec<String> = kept.into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["files".to_string(), "grep".to_string()]);
    }

    #[test]
    fn a_wildcard_allowlist_matches_a_family() {
        // A profile that wants "every MCP tool from this server" should not have
        // to enumerate them, and they change when the server does.
        let all = vec![spec("mcp__github__create_issue"), spec("files")];
        let kept = AgentTool::permitted(all, &["mcp__github__*".to_string()]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name, "mcp__github__create_issue");
    }

    #[test]
    fn a_star_allowlist_permits_everything() {
        let all = vec![spec("files"), spec("terminal")];
        assert_eq!(AgentTool::permitted(all, &["*".to_string()]).len(), 2);
    }

    #[test]
    fn an_unknown_name_is_not_matched_by_a_prefix_pattern() {
        assert!(!glob_matches("files*", "grep"));
        assert!(glob_matches("files*", "files"));
        assert!(!glob_matches("files", "filesx"));
    }

    #[tokio::test]
    async fn lists_profiles_when_no_name_is_given() {
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(EchoRunner));
        let result = tool
            .invoke(ctx(), serde_json::json!({}))
            .await
            .expect("invoke");
        assert!(result.ok);
        let agents = result.content["agents"].as_array().expect("array");
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0]["name"], "explorer", "listed in source order");
    }

    #[tokio::test]
    async fn delegates_and_returns_the_conclusion() {
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(EchoRunner));
        let result = tool
            .invoke(
                ctx(),
                serde_json::json!({ "agent": "explorer", "task": "find the auth module" }),
            )
            .await
            .expect("invoke");
        assert!(result.ok, "{:?}", result.error);
        assert_eq!(result.content["text"], "handled: find the auth module");
        assert_eq!(result.content["steps"], 3);
        assert_eq!(result.content["tool_calls"][0], "grep");
    }

    #[tokio::test]
    async fn a_failed_sub_run_is_reported_to_the_parent() {
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(FailingRunner));
        let result = tool
            .invoke(
                ctx(),
                serde_json::json!({ "agent": "explorer", "task": "go" }),
            )
            .await
            .expect("invoke");
        assert!(!result.ok, "the parent must see the failure");
        assert!(result.error.unwrap().contains("out of budget"));
    }

    #[tokio::test]
    async fn an_unknown_profile_lists_the_alternatives() {
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(EchoRunner));
        let result = tool
            .invoke(ctx(), serde_json::json!({ "agent": "nope", "task": "go" }))
            .await
            .expect("invoke");
        assert!(!result.ok);
        assert_eq!(result.content["available"][0], "explorer");
    }

    #[tokio::test]
    async fn a_missing_task_is_rejected() {
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(EchoRunner));
        let result = tool
            .invoke(ctx(), serde_json::json!({ "agent": "explorer" }))
            .await
            .expect("invoke");
        assert!(!result.ok);
        assert!(result.error.as_deref().is_some_and(|e| e.contains("task")));
    }

    #[tokio::test]
    async fn tasks_fan_out_and_each_result_carries_its_task() {
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(EchoRunner));
        let result = tool
            .invoke(
                ctx(),
                serde_json::json!({
                    "agent": "explorer",
                    "tasks": ["first", "second", "third"]
                }),
            )
            .await
            .expect("invoke");
        assert!(result.ok);
        let results = result.content["results"].as_array().expect("results");
        assert_eq!(results.len(), 3);
        let mut tasks: Vec<&str> = results.iter().filter_map(|r| r["task"].as_str()).collect();
        tasks.sort_unstable();
        assert_eq!(tasks, vec!["first", "second", "third"]);
    }

    #[tokio::test]
    async fn fan_out_rejects_more_than_the_total_cap() {
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(EchoRunner));
        let tasks: Vec<String> = (0..MAX_TOTAL_DELEGATES + 1)
            .map(|i| format!("task-{i}"))
            .collect();
        let result = tool
            .invoke(
                ctx(),
                serde_json::json!({ "agent": "explorer", "tasks": tasks }),
            )
            .await
            .expect("invoke");
        assert!(!result.ok, "unbounded fan-out must be refused");
        assert_eq!(result.content["requested_tasks"], MAX_TOTAL_DELEGATES + 1);
        assert_eq!(result.content["max_tasks"], MAX_TOTAL_DELEGATES);
        assert!(result.error.unwrap().contains("at most 32 per delegation"));
    }

    #[tokio::test]
    async fn an_empty_profile_list_is_reported_clearly() {
        let tool = AgentTool::new(Arc::new(StaticAgents(Vec::new())), Arc::new(EchoRunner));
        let result = tool
            .invoke(ctx(), serde_json::json!({}))
            .await
            .expect("invoke");
        assert!(!result.ok);
        assert!(result.error.unwrap().contains("no agent profiles"));
    }

    #[tokio::test]
    async fn a_schema_returns_a_parsed_structured_copy_alongside_text() {
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(JsonRunner));
        let result = tool
            .invoke(
                ctx(),
                serde_json::json!({
                    "agent": "explorer",
                    "task": "pick a number",
                    "schema": { "type": "object" },
                }),
            )
            .await
            .expect("invoke");
        assert!(result.ok, "{:?}", result.error);
        assert_eq!(result.content["structured"]["answer"], 42);
        assert!(
            result.content["text"]
                .as_str()
                .is_some_and(|t| t.contains("42")),
            "raw text is always kept"
        );
    }

    #[tokio::test]
    async fn prose_with_a_schema_reports_text_only() {
        // A child that ignores the response contract is not a new failure:
        // `structured` stays absent and the parent reads `text`.
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(EchoRunner));
        let result = tool
            .invoke(
                ctx(),
                serde_json::json!({
                    "agent": "explorer",
                    "task": "find the auth module",
                    "schema": { "type": "object" },
                }),
            )
            .await
            .expect("invoke");
        assert!(result.ok, "{:?}", result.error);
        assert!(result.content.get("structured").is_none());
    }

    #[tokio::test]
    async fn fan_out_aggregates_child_budgets_and_merge_state() {
        let tool = AgentTool::new(Arc::new(StaticAgents(profiles())), Arc::new(EchoRunner));
        let result = tool
            .invoke(
                ctx(),
                serde_json::json!({
                    "agent": "explorer",
                    "tasks": [
                        "first",
                        { "task": "second", "schema": { "type": "string" } }
                    ],
                    "isolate": true,
                    "auto_merge": true,
                }),
            )
            .await
            .expect("invoke");
        assert!(result.ok, "{:?}", result.error);
        let budget = &result.content["child_budget"];
        assert_eq!(budget["tasks"], 2);
        // EchoRunner reports 3 steps and one tool call per child.
        assert_eq!(budget["steps_total"], 6);
        assert_eq!(budget["tool_calls_total"], 2);
        assert_eq!(result.content["merge_conflicts"], serde_json::json!([]));
        assert_eq!(result.content["merge_ready"], true);
    }

    #[test]
    fn structured_parsing_accepts_fenced_json() {
        let value = parse_structured("```json\n{\"a\": 1}\n```").expect("fenced");
        assert_eq!(value["a"], 1);
        assert!(parse_structured("just prose").is_none());
    }
}
