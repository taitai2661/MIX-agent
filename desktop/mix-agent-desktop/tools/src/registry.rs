//! Tool Registry: holds the canonical tool list per Run mode.
//!
//! Besides the built-in Tools the registry routes two dynamic families:
//!
//! - `browser` — handled by the Browser Operator (`mixd-browser`).
//! - `mcp__<connection>__<tool>` — handled by the MCP host (`mixd-mcp`).

use std::collections::HashMap;
use std::sync::Arc;

use crate::agent::{AgentRunner, AgentSource, AgentTool};
use crate::edit::EditTool;
use crate::egress::EgressPolicy;
use crate::files::FilesTool;
use crate::image_vision::ImageVisionTool;
use crate::network::{NetworkTool, WebSearchTool};
use crate::search::{GlobTool, GrepTool};
use crate::skill::{SkillSource, SkillTool};
use crate::terminal::TerminalTool;
use crate::todo::{TodoStore, TodoTool};
use crate::user_plan::UserPlanTool;
use crate::web::WebFetchTool;
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
}

impl ToolRegistry {
    pub fn new(egress: EgressPolicy) -> Self {
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let network = Arc::new(NetworkTool::new(egress.clone()));
        tools.insert("network".to_string(), network.clone());
        tools.insert("web_search".to_string(), Arc::new(WebSearchTool));
        tools.insert(
            "web_fetch".to_string(),
            Arc::new(WebFetchTool::new(NetworkTool::new(egress.clone()))),
        );
        tools.insert("files".to_string(), Arc::new(FilesTool));
        tools.insert("edit".to_string(), Arc::new(EditTool));
        tools.insert("glob".to_string(), Arc::new(GlobTool));
        tools.insert("grep".to_string(), Arc::new(GrepTool));
        tools.insert("terminal".to_string(), Arc::new(TerminalTool::default()));
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
    pub fn specs(&self) -> Vec<crate::ToolSpec> {
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
            out.push(crate::ToolSpec {
                name: "browser".to_string(),
                description: spec["description"].as_str().unwrap_or_default().to_string(),
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
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {}", name))?;
        tool.invoke(ctx, args).await
    }
}

/// Translate the flat JSON arguments the model produces into a typed action.
fn browser_action(args: &serde_json::Value) -> anyhow::Result<mixd_browser::BrowserAction> {
    use mixd_browser::BrowserAction as A;
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("browser action is required"))?;
    let text = |key: &str| {
        args.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let number = |key: &str| args.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let condition = || -> anyhow::Result<mixd_browser::WaitCondition> {
        Ok(
            match args
                .get("condition")
                .and_then(|v| v.as_str())
                .unwrap_or("selector")
            {
                "url" => mixd_browser::WaitCondition::Url { url: text("url") },
                "function" => mixd_browser::WaitCondition::Function {
                    script: text("script"),
                },
                _ => mixd_browser::WaitCondition::Selector {
                    selector: text("selector"),
                },
            },
        )
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
            timeout_ms: args
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(10_000),
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
}
