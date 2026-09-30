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
}

/// A finished sub-run, handed back to the parent.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DelegateResult {
    pub agent: String,
    pub text: String,
    pub steps: u32,
    pub tool_calls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
        "Delegate a self-contained task to a saved agent profile. The sub-agent gets its \
         own context and budget and returns only its conclusion, so use it for open-ended \
         exploration rather than for steps you could do yourself. Call with no arguments \
         to list the available profiles."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent": { "type": "string", "description": "Profile name. Omit to list them." },
                "task": {
                    "type": "string",
                    "description": "The full task, including everything the sub-agent needs. It cannot see this conversation."
                }
            },
            "required": ["task"]
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
        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| anyhow::anyhow!("task is required when delegating"))?;

        match self.runner.delegate(&ctx, profile, task).await {
            Ok(result) => {
                let failed = result.error.is_some();
                Ok(ToolResult {
                    ok: !failed,
                    content: serde_json::to_value(&result)
                        .unwrap_or_else(|_| serde_json::json!({})),
                    error: result.error,
                })
            }
            Err(err) => Ok(ToolResult {
                ok: false,
                content: serde_json::json!({ "agent": profile.name }),
                error: Some(format!("delegation failed: {err}")),
            }),
        }
    }
}

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
            })
        }
    }

    struct FailingRunner;

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
            },
            AgentProfile {
                id: "2".into(),
                name: "builder".into(),
                instructions: "Makes changes.".into(),
                tool_ids: vec!["files".into(), "edit".into(), "terminal".into()],
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
        assert!(tool
            .invoke(ctx(), serde_json::json!({ "agent": "explorer" }))
            .await
            .is_err());
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
}
