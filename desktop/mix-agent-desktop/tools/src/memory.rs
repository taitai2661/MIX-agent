//! Agent-facing Memory / Knowledge tool.
//!
//! The actual recall/store logic lives in `mixd-core::memory` and
//! `mixd-core::retrieval`, which the `tools` crate cannot depend on (core
//! depends on tools, not the other way around). The tool therefore dispatches
//! through [`MemoryDispatch`], implemented by the app crate — the same pattern
//! as [`crate::McpDispatch`].

use std::sync::Arc;

use async_trait::async_trait;

use super::{Tool, ToolContext, ToolResult};

#[async_trait]
pub trait MemoryDispatch: Send + Sync {
    /// Lexical recall over stored Memory items.
    async fn recall(&self, query: &str, limit: usize) -> anyhow::Result<serde_json::Value>;
    /// Persist a durable Memory item.
    async fn remember(
        &self,
        title: &str,
        content: &str,
        scope: &str,
    ) -> anyhow::Result<serde_json::Value>;
    /// Remove a Memory item by id.
    async fn forget(&self, id: &str) -> anyhow::Result<serde_json::Value>;
    /// Lexical search over uploaded Knowledge documents.
    async fn search_knowledge(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<serde_json::Value>;
}

pub struct MemoryTool {
    dispatch: Arc<dyn MemoryDispatch>,
}

impl MemoryTool {
    pub fn new(dispatch: Arc<dyn MemoryDispatch>) -> Self {
        Self { dispatch }
    }
}

#[async_trait]
impl Tool for MemoryTool {
    fn id(&self) -> &str {
        "memory"
    }
    fn name(&self) -> &str {
        "Memory"
    }
    fn description(&self) -> &str {
        "Recall, store and forget long-term memories, and search the knowledge base. \
         Use `recall` before work that may depend on the user's preferences, `remember` \
         to persist a durable fact, `forget` to remove one by id, and `search_knowledge` \
         for uploaded documents."
    }
    /// Reads are safe; writes are not.
    fn is_sensitive(&self, args: &serde_json::Value) -> bool {
        !matches!(
            args.get("action").and_then(|v| v.as_str()),
            Some("recall") | Some("search_knowledge")
        )
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["recall", "remember", "forget", "search_knowledge"]
                },
                "query": { "type": "string" },
                "title": { "type": "string" },
                "content": { "type": "string" },
                "scope": { "type": "string", "description": "user | project | task" },
                "id": { "type": "string", "description": "Memory id to forget" },
                "limit": { "type": "integer", "default": 8 }
            },
            "required": ["action"]
        })
    }
    /// `forget` targets a specific Memory id so a pattern rule can scope it.
    fn pattern_target(&self, args: &serde_json::Value) -> Option<String> {
        args.get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }
    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(8)
            .clamp(1, 25) as usize;
        match action {
            "recall" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                if query.trim().is_empty() {
                    return Ok(ToolResult::fail("query is required for recall"));
                }
                Ok(ToolResult::ok(self.dispatch.recall(query, limit).await?))
            }
            "search_knowledge" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                if query.trim().is_empty() {
                    return Ok(ToolResult::fail("query is required for search_knowledge"));
                }
                Ok(ToolResult::ok(
                    self.dispatch.search_knowledge(query, limit).await?,
                ))
            }
            "remember" => {
                let title = args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if title.is_empty() || content.is_empty() {
                    return Ok(ToolResult::fail(
                        "title and content are required for remember",
                    ));
                }
                let scope = args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .unwrap_or("user");
                Ok(ToolResult::ok(
                    self.dispatch.remember(title, content, scope).await?,
                ))
            }
            "forget" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if id.is_empty() {
                    return Ok(ToolResult::fail("id is required for forget"));
                }
                Ok(ToolResult::ok(self.dispatch.forget(id).await?))
            }
            other => Ok(ToolResult::fail(format!("unknown memory action: {other}"))),
        }
    }
}
