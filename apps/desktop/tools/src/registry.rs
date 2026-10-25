//! Tool Registry: holds the canonical tool list per Run mode.
//!
//! Besides the built-in Tools the registry routes two dynamic families:
//!
//! - `browser` — handled by the Browser Operator (`mixd-browser`).
//! - `mcp__<connection>__<tool>` — handled by the MCP host (`mixd-mcp`).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::agent::{AgentRunner, AgentSource, AgentTool};
use crate::edit::EditTool;
use crate::egress::EgressPolicy;
use crate::files::FilesTool;
use crate::git::GitTool;
use crate::image_vision::ImageVisionTool;
use crate::knowledge::{KnowledgeDispatch, KnowledgeTool};
use crate::memory::{MemoryDispatch, MemoryTool};
use crate::network::{NetworkTool, WebSearchTool};
use crate::pdf::WebFetchPdfTool;
use crate::search::{GlobTool, GrepTool};
use crate::skill::{SkillSource, SkillTool};
use crate::symbols::SymbolsTool;
use crate::terminal::TerminalTool;
use crate::todo::{TodoStore, TodoTool};
use crate::user_plan::UserPlanTool;
use crate::web::WebFetchTool;
use crate::workspace_check::WorkspaceCheckTool;
use crate::{Tool, ToolContext, ToolResult};

/// MCP-backed Tools are dispatched through this hook rather than the static
/// registry, because the live list depends on which servers are connected.
#[async_trait::async_trait]
pub trait McpDispatch: Send + Sync {
    async fn dispatch(
        &self,
        qualified_name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<ToolResult>;
}

#[derive(Clone)]
pub struct ToolRegistry {
    pub egress: EgressPolicy,
    pub tools: HashMap<String, Arc<dyn Tool>>,
    pub browser: Option<Arc<mixd_browser::BrowserOperator>>,
    pub mcp: Option<Arc<dyn McpDispatch>>,
    /// Checklist shared with the Run Engine so it can re-render the list each turn.
    pub todos: TodoStore,
    /// Live MCP tool specs, so the engine can advertise what the host exposes.
    mcp_specs: Arc<Vec<crate::ToolSpec>>,
    /// The skill catalogue, so the engine can list it in the system prompt.
    skill_tool: Option<Arc<SkillTool>>,
    /// Held typed as well so the caches can be dropped when the user edits a
    /// profile; a `dyn Tool` cannot be downcast back.
    agent_tool: Option<Arc<AgentTool>>,
    /// The advertised spec list, built on first use.
    ///
    /// Materialising it asks every tool to write out its JSON schema, and the
    /// list is re-read on every run, every sub-agent delegation and every
    /// diagnostics call — so the schemas were rebuilt from scratch each time.
    specs_cache: OnceLock<Arc<Vec<crate::ToolSpec>>>,
}

impl ToolRegistry {
    pub fn new(egress: EgressPolicy) -> Self {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let network = Arc::new(NetworkTool::new(egress.clone()));
        tools.insert("network".to_string(), network.clone());
        tools.insert(
            "web_search".to_string(),
            Arc::new(WebSearchTool::new(egress.clone())),
        );
        tools.insert(
            "web_fetch_pdf".to_string(),
            Arc::new(WebFetchPdfTool::new(egress.clone())),
        );
        tools.insert(
            "web_fetch".to_string(),
            Arc::new(WebFetchTool::new(NetworkTool::new(egress.clone()))),
        );
        tools.insert("files".to_string(), Arc::new(FilesTool));
        tools.insert("edit".to_string(), Arc::new(EditTool));
        tools.insert("glob".to_string(), Arc::new(GlobTool));
        tools.insert("grep".to_string(), Arc::new(GrepTool));
        tools.insert("terminal".to_string(), Arc::new(TerminalTool::default()));
        tools.insert("symbols".to_string(), Arc::new(SymbolsTool));
        tools.insert("git".to_string(), Arc::new(GitTool));
        tools.insert("workspace_check".to_string(), Arc::new(WorkspaceCheckTool));
        tools.insert("image_vision".to_string(), Arc::new(ImageVisionTool));
        tools.insert("user_plan".to_string(), Arc::new(UserPlanTool));
        // One store shared with the registry, not a second one: the engine
        // re-renders `registry.todos` into the head block each turn, and two
        // stores would mean the checklist the model writes is never the one it
        // can see.
        let todos = TodoStore::new();
        tools.insert(
            "todo".to_string(),
            Arc::new(TodoTool {
                store: todos.clone(),
            }),
        );
        Self {
            egress,
            tools,
            browser: None,
            mcp: None,
            todos,
            mcp_specs: Arc::new(Vec::new()),
            skill_tool: None,
            agent_tool: None,
            specs_cache: OnceLock::new(),
        }
    }

    /// Attach the skill catalogue so the `skill` Tool becomes available.
    pub fn with_skills(mut self, source: Arc<dyn SkillSource>) -> Self {
        let tool = Arc::new(SkillTool::new(source));
        self.tools.insert(tool.id().to_string(), tool.clone());
        self.skill_tool = Some(tool);
        self
    }

    /// Attach the agent-profile source and runner so `agent` becomes available.
    pub fn with_agents(
        mut self,
        source: Arc<dyn AgentSource>,
        runner: Arc<dyn AgentRunner>,
    ) -> Self {
        let tool = Arc::new(AgentTool::new(source, runner));
        self.tools.insert("agent".to_string(), tool.clone());
        self.agent_tool = Some(tool);
        self
    }

    /// Attach the Memory / Knowledge tool so the agent can recall and persist.
    pub fn with_memory(mut self, dispatch: Arc<dyn MemoryDispatch>) -> Self {
        self.tools
            .insert("memory".to_string(), Arc::new(MemoryTool::new(dispatch)));
        self
    }

    /// Attach the writable Knowledge store (`add` / `search` / `delete` /
    /// `clip`). Search stays on `memory` as well; this is the write surface.
    pub fn with_knowledge(mut self, dispatch: Arc<dyn KnowledgeDispatch>) -> Self {
        let tool = Arc::new(KnowledgeTool::new(dispatch, self.egress.clone()));
        self.tools.insert(tool.id().to_string(), tool);
        self
    }

    /// The skill catalogue rendered for the system prompt, or an empty string.
    ///
    /// `root` scopes the catalogue to one project folder: project skills are
    /// discovered there, so a catalogue built for another root would list
    /// skills this run cannot see.
    pub async fn skills_prompt(&self, root: Option<&str>) -> String {
        match &self.skill_tool {
            Some(tool) => tool.catalogue_prompt(root).await,
            None => String::new(),
        }
    }

    /// Drop cached skill and agent catalogues so a newly saved one is visible
    /// to the next run without restarting the app.
    pub async fn invalidate_catalogues(&self) {
        if let Some(tool) = &self.skill_tool {
            tool.invalidate().await;
        }
        if let Some(tool) = &self.agent_tool {
            tool.invalidate().await;
        }
    }

    /// Attach the Browser Operator so the `browser` Tool becomes available.
    pub fn with_browser(mut self, operator: Arc<mixd_browser::BrowserOperator>) -> Self {
        self.browser = Some(operator);
        self
    }

    /// Attach the MCP host so `mcp__<connection>__<tool>` becomes available.
    pub fn with_mcp(mut self, mcp: Arc<dyn McpDispatch>) -> Self {
        self.mcp = Some(mcp);
        self
    }

    /// Record the tools the MCP host currently exposes.
    ///
    /// The host connects and disconnects independently of the registry, so the
    /// advertised spec list has to be pushed in explicitly; without this the
    /// `mcp__*` tools are dispatchable but the model is never told they exist.
    pub fn set_mcp_specs(&mut self, specs: Vec<crate::ToolSpec>) {
        self.mcp_specs = Arc::new(specs);
        // The advertised list just changed, so the memoised one is stale.
        self.specs_cache.take();
    }

    pub fn mcp_specs(&self) -> Vec<crate::ToolSpec> {
        self.mcp_specs.as_ref().clone()
    }

    pub fn list(&self) -> Vec<Arc<dyn Tool>> {
        let mut out: Vec<Arc<dyn Tool>> = self.tools.values().cloned().collect();
        out.sort_by(|a, b| a.id().cmp(b.id()));
        out
    }

    /// Tool specs the Run Engine advertises, including Browser and MCP Tools.
    ///
    /// The returned `Arc` is shared: callers read it without rebuilding, and a
    /// run hands the very same list to every one of its provider turns.
    pub fn specs(&self) -> Arc<Vec<crate::ToolSpec>> {
        self.specs_cache
            .get_or_init(|| Arc::new(self.build_specs()))
            .clone()
    }

    /// Mode-filtered specs, mirroring Docker's `allowed_modes` boundary.
    ///
    /// Takes `&str` (not `mixd_core::Mode`) on purpose: `mixd-core` depends on
    /// this crate, so importing its `Mode` here would be a dependency cycle.
    /// The typed boundary lives one layer up
    /// (`RunEngine::tool_specs_for_mode(Mode)`); unknown strings fall back to
    /// the thinking/plan set with a warning rather than silently.
    /// Docker's chat/thinking runs exclude delegation and long-work planning:
    /// `update_plan` is agent-only, and delegation is agent-only. The Desktop
    /// equivalents are `todo` (= checklist) and `agent` (= sub-delegate);
    /// `user_plan` blocks on user confirmation and is noise in a quick chat.
    /// Thinking keeps `todo`/`user_plan` (the built-in procedure references the
    /// checklist) but still cannot delegate. Plan is read-only (enforced in the
    /// engine, not by withholding read tools) but must not delegate either: a
    /// sub-agent would run outside the read-only refusal.
    pub fn specs_for_mode(&self, mode: &str) -> Arc<Vec<crate::ToolSpec>> {
        if mode == "agent" {
            return self.specs();
        }
        if !matches!(mode, "chat" | "thinking" | "plan") {
            tracing::warn!(
                mode,
                "unknown mode for tool specs; using the thinking/plan set"
            );
        }
        let excluded: &[&str] = if mode == "chat" {
            &["agent", "todo", "user_plan"]
        } else {
            // thinking + plan: no delegation; plan keeps its read tools and
            // checklist, the engine refuses mutating calls.
            &["agent"]
        };
        let full = self.specs();
        Arc::new(
            full.iter()
                .filter(|s| !excluded.contains(&s.name.as_str()))
                .cloned()
                .collect(),
        )
    }

    fn build_specs(&self) -> Vec<crate::ToolSpec> {
        let mut out: Vec<crate::ToolSpec> = self
            .list()
            .iter()
            .map(|t| crate::ToolSpec {
                name: t.id().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
            })
            .collect();
        if self.browser.is_some() {
            let spec = mixd_browser::browser_tool_spec();
            let description = spec["description"].as_str().unwrap_or_else(|| {
                tracing::debug!("browser tool spec has no description; advertising empty");
                ""
            });
            out.push(crate::ToolSpec {
                name: "browser".to_string(),
                description: description.to_string(),
                parameters: spec["parameters"].clone(),
            });
        }
        out.extend(self.mcp_specs.iter().cloned());
        out
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(id).cloned()
    }

    pub async fn dispatch(
        &self,
        ctx: ToolContext,
        name: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        tracing::debug!(tool = %name, run = %ctx.run_id, "tool dispatch started");
        let outcome = self.dispatch_inner(ctx, name, args).await;
        match &outcome {
            Ok(result) if result.ok => {}
            Ok(result) => tracing::warn!(
                tool = %name,
                error = result.error.as_deref().unwrap_or(""),
                "tool reported a failure"
            ),
            Err(err) => tracing::warn!(tool = %name, %err, "tool dispatch failed"),
        }
        outcome
    }

    async fn dispatch_inner(
        &self,
        ctx: ToolContext,
        name: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        if name == "browser" {
            let Some(operator) = &self.browser else {
                return Err(anyhow::anyhow!("browser backend is not available"));
            };
            let action = browser_action(&args)?;
            // The browser is a second execution channel that used to bypass the
            // egress policy entirely. At minimum, gate direct navigation on the
            // same allowlist + private-IP deny the HTTP tools use. `wait_for`
            // with a URL condition is gated the same way: without it the URL
            // poll would be the hole the navigate gate closed (the backend
            // re-checks with DNS resolution before dialling; this parse-time
            // literal check fails fast on the obvious shapes).
            if let mixd_browser::BrowserAction::Navigate { url } = &action {
                self.egress.resolve_async(url).await?;
                mixd_browser::reject_private_navigate_url(url)?;
            }
            if let mixd_browser::BrowserAction::WaitFor {
                condition: mixd_browser::WaitCondition::Url { url },
                ..
            } = &action
            {
                self.egress.resolve_async(url).await?;
                mixd_browser::reject_private_navigate_url(url)?;
            }
            let response = operator
                .invoke(mixd_browser::BrowserCall {
                    workspace_id: ctx.workspace_id,
                    action,
                })
                .await?;
            return Ok(ToolResult {
                ok: response.ok,
                content: response.payload,
                error: response.error,
            });
        }
        if name.starts_with("mcp__") {
            let Some(mcp) = &self.mcp else {
                return Err(anyhow::anyhow!("MCP host is not available"));
            };
            return mcp.dispatch(name, args).await;
        }
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {}", name))?;
        tool.invoke(ctx, args).await
    }

    /// The string a `pattern`-scoped rule should match against.
    ///
    /// Wraps the tool lookup so callers (the Run Engine in particular) can ask
    /// for the right target without knowing whether the tool is a builtin,
    /// `browser`, or an `mcp__<conn>__<tool>` proxy.
    pub fn pattern_target_for(&self, name: &str, args: &serde_json::Value) -> Option<String> {
        if name == "browser" {
            // The browser tool's call surface is a single `action` plus
            // target fields. Expose the action together with the URL (for
            // `navigate` / `wait_for(url)`) so a rule can match a specific
            // site (`pattern = "navigate*example.com*"`) instead of only the
            // bare action, which could never distinguish one site from another.
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if action.is_empty() && url.is_empty() {
                return None;
            }
            if url.is_empty() {
                return Some(action.to_string());
            }
            return Some(format!("{action} {url}"));
        }
        if let Some(rest) = name.strip_prefix("mcp__") {
            // The connection id is the only stable per-server identifier the
            // model can target, so use it instead of the inner tool name
            // (which the model never has to spell).
            let connection_id = rest.split("__").next().unwrap_or(rest);
            return Some(connection_id.to_string());
        }
        self.tools
            .get(name)
            .and_then(|tool| tool.pattern_target(args))
    }
}

/// Translate the flat JSON arguments the model produces into a typed action.
fn browser_action(args: &serde_json::Value) -> anyhow::Result<mixd_browser::BrowserAction> {
    use mixd_browser::BrowserAction as A;
    let action = crate::args::str_arg(args, "action")
        .ok_or_else(|| anyhow::anyhow!("browser action is required"))?;
    let text = |key: &str| crate::args::str_or(args, key, "").to_string();
    // Missing/invalid numbers fall back to 0 (origin scroll): logged at debug
    // so a malformed model argument is visible without failing the turn.
    let number = |key: &str| {
        let v = args.get(key).and_then(|v| v.as_i64()).unwrap_or_else(|| {
            if args.get(key).is_some() {
                tracing::debug!(key, "browser numeric arg is not an integer; using 0");
            }
            0
        }) as i32;
        v
    };
    let condition = || -> anyhow::Result<mixd_browser::WaitCondition> {
        Ok(match crate::args::str_or(args, "condition", "selector") {
            "url" => mixd_browser::WaitCondition::Url { url: text("url") },
            "function" => mixd_browser::WaitCondition::Function {
                script: text("script"),
            },
            _ => mixd_browser::WaitCondition::Selector {
                selector: text("selector"),
            },
        })
    };
    Ok(match action {
        "navigate" => A::Navigate { url: text("url") },
        "click" => A::Click {
            selector: text("selector"),
        },
        "type" => A::Type {
            selector: text("selector"),
            text: text("text"),
        },
        "scroll" => A::Scroll {
            x: number("x"),
            y: number("y"),
        },
        "dom" => A::Dom {
            selector: args
                .get("selector")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        },
        "screenshot" => A::Screenshot {
            full_page: args
                .get("full_page")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        },
        "evaluate" => A::Evaluate {
            script: text("script"),
        },
        "wait_for" => A::WaitFor {
            condition: condition()?,
            // Default 10 s documented in the tool schema; an invalid value is
            // logged and clamped to it rather than failing the call.
            timeout_ms: args
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(|| {
                    if args.get("timeout_ms").is_some() {
                        tracing::debug!(
                            "browser timeout_ms is not a positive integer; using 10_000"
                        );
                    }
                    10_000
                }),
        },
        "info" => A::Info,
        "close" => A::Close,
        other => return Err(anyhow::anyhow!("unknown browser action: {}", other)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::SkillSummary;

    #[test]
    fn parses_navigate() {
        let action = browser_action(
            &serde_json::json!({ "action": "navigate", "url": "https://example.com" }),
        )
        .expect("parse");
        assert!(
            matches!(action, mixd_browser::BrowserAction::Navigate { url } if url == "https://example.com")
        );
    }

    #[test]
    fn parses_wait_for_with_condition() {
        let action = browser_action(&serde_json::json!({
            "action": "wait_for",
            "condition": "selector",
            "selector": "#done",
            "timeout_ms": 5000,
        }))
        .expect("parse");
        match action {
            mixd_browser::BrowserAction::WaitFor {
                condition,
                timeout_ms,
            } => {
                assert_eq!(timeout_ms, 5000);
                assert!(
                    matches!(condition, mixd_browser::WaitCondition::Selector { selector } if selector == "#done")
                );
            }
            other => panic!("unexpected action: {:?}", other),
        }
    }

    #[test]
    fn rejects_unknown_action() {
        assert!(browser_action(&serde_json::json!({ "action": "teleport" })).is_err());
    }

    #[test]
    fn private_literal_ips_are_rejected_at_parse_time_for_navigate_and_wait_for_url() {
        // Parse-time (sync) literal gate; the backend re-checks with DNS
        // resolution before dialling. `dispatch_inner` enforces both, so the
        // parser staying total here is intentional — this only documents the
        // gate location, the enforcement lives in `dispatch_inner`.
        for args in [
            serde_json::json!({ "action": "navigate", "url": "http://127.0.0.1/" }),
            serde_json::json!({ "action": "navigate", "url": "http://169.254.169.254/" }),
        ] {
            let action = browser_action(&args).expect("navigate parses");
            match action {
                mixd_browser::BrowserAction::Navigate { url } => {
                    assert!(mixd_browser::reject_private_navigate_url(&url).is_err())
                }
                other => panic!("unexpected action: {:?}", other),
            }
        }
        let action = browser_action(&serde_json::json!({
            "action": "wait_for",
            "condition": "url",
            "url": "http://10.0.0.1/",
        }))
        .expect("wait_for parses");
        match action {
            mixd_browser::BrowserAction::WaitFor { condition, .. } => match condition {
                mixd_browser::WaitCondition::Url { url } => {
                    assert!(mixd_browser::reject_private_navigate_url(&url).is_err())
                }
                other => panic!("unexpected condition: {:?}", other),
            },
            other => panic!("unexpected action: {:?}", other),
        }
    }

    struct OneSkill;

    #[async_trait::async_trait]
    impl SkillSource for OneSkill {
        async fn skills(&self, _root: Option<&str>) -> anyhow::Result<Vec<SkillSummary>> {
            Ok(vec![SkillSummary {
                name: "expense-report".into(),
                description: "Build expense reports".into(),
                body_markdown: "1. Collect receipts".into(),
                allowed_tools: Vec::new(),
            }])
        }
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

    #[tokio::test]
    async fn skill_is_registered_and_dispatchable() {
        let registry = ToolRegistry::new(EgressPolicy::default()).with_skills(Arc::new(OneSkill));
        assert!(registry.get("skill").is_some(), "skill tool not registered");
        assert!(
            registry.specs().iter().any(|s| s.name == "skill"),
            "skill spec not advertised"
        );
        let result = registry
            .dispatch(ctx(), "skill", serde_json::json!({}))
            .await
            .expect("dispatch");
        assert!(result.ok, "skill dispatch failed: {:?}", result.error);
        assert_eq!(result.content["skills"][0]["name"], "expense-report");
    }

    #[test]
    fn chat_specs_exclude_delegation_and_planning_but_keep_search_and_files() {
        let registry = ToolRegistry::new(EgressPolicy::default());
        let specs = registry.specs_for_mode("chat");
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(
            !names.is_empty(),
            "chat must now carry tools (Docker parity)"
        );
        for excluded in ["agent", "todo", "user_plan"] {
            assert!(
                !names.contains(&excluded),
                "{excluded} must stay agent-only"
            );
        }
        for required in [
            "files",
            "edit",
            "glob",
            "grep",
            "terminal",
            "workspace_check",
            "web_search",
            "web_fetch",
            "web_fetch_pdf",
            "network",
        ] {
            assert!(names.contains(&required), "{required} must be in chat");
        }
    }

    #[test]
    fn thinking_keeps_the_checklist_but_still_cannot_delegate() {
        let registry = ToolRegistry::new(EgressPolicy::default());
        let specs = registry.specs_for_mode("thinking");
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"agent"));
        assert!(names.contains(&"todo"));
    }

    #[test]
    fn agent_mode_keeps_everything() {
        let registry = ToolRegistry::new(EgressPolicy::default());
        assert_eq!(
            registry.specs_for_mode("agent").len(),
            registry.specs().len()
        );
    }
}
