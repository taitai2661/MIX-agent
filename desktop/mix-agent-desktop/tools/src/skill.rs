//! Skill retrieval.
//!
//! The catalogue of `SKILL.md` procedures is injected into the system prompt as
//! names and descriptions only; the body is fetched here, on demand. That split
//! is the whole point: a library of twenty skills should cost a few hundred
//! tokens, not twenty documents.
//!
//! The tool delegates the storage lookup to the host because the tools crate has
//! no database handle of its own beyond the approval tables.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::{Tool, ToolContext, ToolResult};

/// A skill as the tools crate sees it: the fields the prompt needs, nothing more.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub body_markdown: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

/// Supplies the catalogue. Implemented by the app shell, which owns the database.
#[async_trait]
pub trait SkillSource: Send + Sync {
    async fn skills(&self) -> anyhow::Result<Vec<SkillSummary>>;
}

pub struct SkillTool {
    source: Arc<dyn SkillSource>,
    cache: Arc<RwLock<Option<Vec<SkillSummary>>>>,
}

impl SkillTool {
    pub fn new(source: Arc<dyn SkillSource>) -> Self {
        Self {
            source,
            cache: Arc::new(RwLock::new(None)),
        }
    }

    /// The catalogue, cached for the process.
    ///
    /// Skills change only when the user edits them, and a run may call this
    /// many times; re-reading every row per call would be pure overhead.
    pub async fn catalogue(&self) -> anyhow::Result<Vec<SkillSummary>> {
        if let Some(hit) = self.cache.read().await.as_ref() {
            return Ok(hit.clone());
        }
        let loaded = self.source.skills().await?;
        *self.cache.write().await = Some(loaded.clone());
        Ok(loaded)
    }

    /// Drop the cache, so a newly saved skill is visible to the next run.
    pub async fn invalidate(&self) {
        *self.cache.write().await = None;
    }

    /// The catalogue rendered for the system prompt, as `core::skills` does it.
    ///
    /// Duplicated rather than shared because `mixd-tools` cannot depend on
    /// `mixd-core` (core depends on tools). The two are kept in step by the
    /// tests in `core::skills`, which assert the same shape.
    pub async fn catalogue_prompt(&self) -> String {
        let Ok(skills) = self.catalogue().await else {
            return String::new();
        };
        if skills.is_empty() {
            return String::new();
        }
        let mut out = String::from(
            "<skills>\nThe following skills are available. Each has a procedure you have \
             not been shown yet. To use one, call the `skill` tool with its name to read \
             the full instructions before acting.\n",
        );
        for skill in &skills {
            out.push_str(&format!("- {}: {}", skill.name, skill.description));
            if !skill.allowed_tools.is_empty() {
                out.push_str(&format!(
                    " [allowed-tools: {}]",
                    skill.allowed_tools.join(", ")
                ));
            }
            out.push('\n');
        }
        out.push_str("</skills>");
        out
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn id(&self) -> &str {
        "skill"
    }
    /// Reading a procedure the user installed is not a side effect.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }
    fn name(&self) -> &str {
        "Skill"
    }
    fn description(&self) -> &str {
        "Read a skill's full procedure. Call with no name to list what is available. \
         Read a skill before acting on it — the catalogue in your instructions only \
         names them."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name. Omit to list every available skill."
                }
            },
            "required": []
        })
    }

    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let catalogue = self.catalogue().await?;
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if catalogue.is_empty() {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({ "skills": [] }),
                error: Some("no skills are installed".to_string()),
            });
        }
        let name = name.trim();
        if name.is_empty() {
            return Ok(ToolResult {
                ok: true,
                content: serde_json::json!({
                    "skills": catalogue
                        .iter()
                        .map(|s| serde_json::json!({
                            "name": s.name,
                            "description": s.description,
                            "allowed_tools": s.allowed_tools,
                        }))
                        .collect::<Vec<_>>(),
                }),
                error: None,
            });
        }
        match catalogue.iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
            Some(skill) => Ok(ToolResult {
                ok: true,
                content: serde_json::to_value(skill).unwrap_or_else(|_| serde_json::json!({})),
                error: None,
            }),
            None => Ok(ToolResult {
                ok: false,
                content: serde_json::json!({
                    "available": catalogue
                        .iter()
                        .map(|s| s.name.clone())
                        .collect::<Vec<_>>()
                }),
                error: Some(format!("no skill named {name:?}")),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticSource(Vec<SkillSummary>);

    #[async_trait]
    impl SkillSource for StaticSource {
        async fn skills(&self) -> anyhow::Result<Vec<SkillSummary>> {
            Ok(self.0.clone())
        }
    }

    struct CountingSource {
        skills: Vec<SkillSummary>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl SkillSource for CountingSource {
        async fn skills(&self) -> anyhow::Result<Vec<SkillSummary>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.skills.clone())
        }
    }

    fn sample() -> Vec<SkillSummary> {
        vec![
            SkillSummary {
                name: "expense-report".into(),
                description: "Build expense reports from receipts".into(),
                body_markdown: "1. Collect receipts\n2. Categorise".into(),
                allowed_tools: vec!["files".into(), "terminal".into()],
            },
            SkillSummary {
                name: "release-notes".into(),
                description: "Draft release notes from a git log".into(),
                body_markdown: "Run git log".into(),
                allowed_tools: Vec::new(),
            },
        ]
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
    async fn lists_the_catalogue_when_no_name_is_given() {
        let tool = SkillTool::new(Arc::new(StaticSource(sample())));
        let result = tool
            .invoke(ctx(), serde_json::json!({}))
            .await
            .expect("invoke");
        assert!(result.ok);
        let skills = result.content["skills"].as_array().expect("array");
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0]["name"], "expense-report");
        // The listing must stay cheap: no bodies.
        assert!(!result.content.to_string().contains("Categorise"));
    }

    #[tokio::test]
    async fn returns_the_body_for_a_named_skill() {
        let tool = SkillTool::new(Arc::new(StaticSource(sample())));
        let result = tool
            .invoke(ctx(), serde_json::json!({ "name": "expense-report" }))
            .await
            .expect("invoke");
        assert!(result.ok);
        assert!(result.content["body_markdown"]
            .as_str()
            .unwrap_or_default()
            .contains("Categorise"));
    }

    #[tokio::test]
    async fn the_name_match_is_case_insensitive() {
        let tool = SkillTool::new(Arc::new(StaticSource(sample())));
        let result = tool
            .invoke(ctx(), serde_json::json!({ "name": "  Expense-Report " }))
            .await
            .expect("invoke");
        assert!(result.ok, "{:?}", result.error);
    }

    #[tokio::test]
    async fn an_unknown_name_lists_what_is_available() {
        let tool = SkillTool::new(Arc::new(StaticSource(sample())));
        let result = tool
            .invoke(ctx(), serde_json::json!({ "name": "nope" }))
            .await
            .expect("invoke");
        assert!(!result.ok);
        assert_eq!(result.content["available"][0], "expense-report");
    }

    #[tokio::test]
    async fn an_empty_catalogue_reports_clearly() {
        let tool = SkillTool::new(Arc::new(StaticSource(Vec::new())));
        let result = tool
            .invoke(ctx(), serde_json::json!({}))
            .await
            .expect("invoke");
        assert!(!result.ok);
        assert_eq!(tool.catalogue_prompt().await, "");
    }

    #[tokio::test]
    async fn the_catalogue_is_cached_until_invalidated() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tool = SkillTool::new(Arc::new(CountingSource {
            skills: sample(),
            calls: calls.clone(),
        }));
        for _ in 0..5 {
            tool.catalogue().await.expect("catalogue");
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        tool.invalidate().await;
        tool.catalogue().await.expect("catalogue");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "invalidate must force a reload so a new skill is visible"
        );
    }

    #[test]
    fn the_prompt_shows_names_and_tools_but_not_bodies() {
        let tool = SkillTool::new(Arc::new(StaticSource(sample())));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let prompt = runtime.block_on(tool.catalogue_prompt());
        assert!(prompt.contains("expense-report"));
        assert!(prompt.contains("allowed-tools: files, terminal"));
        assert!(!prompt.contains("Categorise"));
    }
}
