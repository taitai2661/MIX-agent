//! Structured task list for multi-step work.
//!
//! Without an external plan, an agent loses track of a five-step refactor the
//! moment the context window compresses. This tool gives it an explicit,
//! inspectable checklist that is re-injected into every turn.
//!
//! The list is per-run and lives in memory: it is derived state, and the
//! authoritative record is the run transcript.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::{Tool, ToolContext, ToolResult};

/// Cap the list so a runaway agent cannot turn its own prompt into noise.
const MAX_ITEMS: usize = 60;
const MAX_ACTIVE_ITEMS: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TodoItem {
    pub id: usize,
    pub content: String,
    pub status: TodoStatus,
}

/// Per-run checklist, shared with the engine so it can re-render the list.
#[derive(Clone)]
pub struct TodoStore {
    items: Arc<Mutex<BTreeMap<usize, TodoItem>>>,
    /// Ids are 1-based so that `0` can mean "not supplied" in tool arguments.
    next_id: Arc<Mutex<usize>>,
}

impl Default for TodoStore {
    fn default() -> Self {
        Self {
            items: Arc::new(Mutex::new(BTreeMap::new())),
            next_id: Arc::new(Mutex::new(1)),
        }
    }
}

impl TodoStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn snapshot(&self) -> Vec<TodoItem> {
        self.items.lock().await.values().cloned().collect()
    }

    pub async fn render(&self) -> String {
        let items = self.snapshot().await;
        if items.is_empty() {
            return String::new();
        }
        let mark = |s: TodoStatus| match s {
            TodoStatus::Pending => "[ ]",
            TodoStatus::InProgress => "[~]",
            TodoStatus::Completed => "[x]",
        };
        let mut out = String::from("<task_list>\n");
        for item in &items {
            out.push_str(&format!(
                "{} {}: {}\n",
                mark(item.status),
                item.id,
                item.content
            ));
        }
        out.push_str("</task_list>");
        out
    }

    pub async fn clear(&self) {
        self.items.lock().await.clear();
    }
}

pub struct TodoTool {
    pub store: TodoStore,
}

impl Default for TodoTool {
    fn default() -> Self {
        Self {
            store: TodoStore::new(),
        }
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn id(&self) -> &str {
        "todo"
    }
    /// The checklist is in-memory and scoped to the run; nothing outside the
    /// session can observe it.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }
    fn name(&self) -> &str {
        "Todo"
    }
    fn description(&self) -> &str {
        "Maintain a checklist for multi-step work. Call it once with the full plan, then \
         update statuses as you go. Keep exactly one item in_progress. The list is shown \
         to the user and re-read on every turn, so use it for anything longer than three steps."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The complete checklist. Send the whole list every time, not just the delta.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "integer", "description": "Stable id; reuse the ids you were given." },
                            "content": { "type": "string", "description": "Imperative description of the work." },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let entries = args
            .get("todos")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("todos must be an array"))?;
        if entries.len() > MAX_ITEMS {
            return Ok(ToolResult {
                ok: false,
                content: serde_json::json!({}),
                error: Some(format!(
                    "at most {MAX_ITEMS} items are allowed, got {}",
                    entries.len()
                )),
            });
        }

        let mut next_id = self.store.next_id.lock().await;
        let mut items = BTreeMap::new();
        for entry in entries {
            let content = entry
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("each todo needs a non-empty content"))?;
            let status = match entry.get("status").and_then(|v| v.as_str()) {
                Some("completed") => TodoStatus::Completed,
                Some("in_progress") => TodoStatus::InProgress,
                _ => TodoStatus::Pending,
            };
            let id = match entry.get("id").and_then(|v| v.as_u64()) {
                Some(raw) if raw > 0 => {
                    let id = raw as usize;
                    *next_id = (*next_id).max(id + 1);
                    id
                }
                _ => {
                    let id = *next_id;
                    *next_id += 1;
                    id
                }
            };
            items.insert(
                id,
                TodoItem {
                    id,
                    content,
                    status,
                },
            );
        }

        // Exactly one item may be in_progress: a list with three "active" items
        // is a list with no active item.
        let mut promoted = false;
        for item in items.values_mut() {
            if item.status == TodoStatus::InProgress {
                if promoted {
                    item.status = TodoStatus::Pending;
                } else {
                    promoted = true;
                }
            }
        }
        let normalised: Vec<TodoItem> = items.values().cloned().collect();
        if normalised.is_empty() {
            self.store.clear().await;
        } else {
            *self.store.items.lock().await = items;
        }

        let remaining = normalised
            .iter()
            .filter(|i| i.status != TodoStatus::Completed)
            .count();
        Ok(ToolResult {
            ok: true,
            content: serde_json::json!({
                "todos": normalised,
                "remaining": remaining,
                "in_progress_cap": MAX_ACTIVE_ITEMS,
            }),
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext {
            run_id: uuid::Uuid::new_v4(),
            workspace_id: uuid::Uuid::new_v4(),
            root_path: None,
            approval_token: None,
        }
    }

    #[tokio::test]
    async fn assigns_ids_and_tracks_status() {
        let tool = TodoTool::default();
        let result = tool
            .invoke(
                ctx(),
                serde_json::json!({ "todos": [
                    { "content": "read the code", "status": "completed" },
                    { "content": "write the fix", "status": "pending" }
                ]}),
            )
            .await
            .expect("invoke");
        assert!(result.ok);
        assert_eq!(result.content["remaining"], 1);
        let items = tool.store.snapshot().await;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].status, TodoStatus::Completed);
        assert_eq!(items[1].id, 2, "ids are allocated sequentially");
    }

    #[tokio::test]
    async fn keeps_explicit_ids_stable_across_updates() {
        let tool = TodoTool::default();
        let first =
            serde_json::json!({ "todos": [{ "id": 1, "content": "a", "status": "pending" }] });
        tool.invoke(ctx(), first).await.expect("invoke");
        let second =
            serde_json::json!({ "todos": [{ "id": 1, "content": "a", "status": "completed" }] });
        tool.invoke(ctx(), second).await.expect("invoke");
        let items = tool.store.snapshot().await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 1);
        assert_eq!(items[0].status, TodoStatus::Completed);
    }

    #[tokio::test]
    async fn demotes_extra_in_progress_items() {
        let tool = TodoTool::default();
        tool.invoke(
            ctx(),
            serde_json::json!({ "todos": [
                { "id": 1, "content": "a", "status": "in_progress" },
                { "id": 2, "content": "b", "status": "in_progress" }
            ]}),
        )
        .await
        .expect("invoke");
        let items = tool.store.snapshot().await;
        assert_eq!(
            items
                .iter()
                .filter(|i| i.status == TodoStatus::InProgress)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn an_empty_list_clears_the_store() {
        let tool = TodoTool::default();
        tool.invoke(
            ctx(),
            serde_json::json!({ "todos": [{ "id": 1, "content": "a", "status": "pending" }] }),
        )
        .await
        .expect("invoke");
        assert!(!tool.store.snapshot().await.is_empty());
        tool.invoke(ctx(), serde_json::json!({ "todos": [] }))
            .await
            .expect("invoke");
        assert!(tool.store.snapshot().await.is_empty());
        assert_eq!(tool.store.render().await, "");
    }

    #[tokio::test]
    async fn rejects_an_oversized_list() {
        let tool = TodoTool::default();
        let todos: Vec<serde_json::Value> = (0..MAX_ITEMS + 1)
            .map(|i| serde_json::json!({ "content": format!("step {i}"), "status": "pending" }))
            .collect();
        let result = tool
            .invoke(ctx(), serde_json::json!({ "todos": todos }))
            .await
            .expect("invoke");
        assert!(!result.ok);
        assert!(result.error.unwrap().contains("at most"));
    }

    #[tokio::test]
    async fn render_uses_checkbox_markers() {
        let tool = TodoTool::default();
        tool.invoke(
            ctx(),
            serde_json::json!({ "todos": [
                { "id": 1, "content": "done", "status": "completed" },
                { "id": 2, "content": "busy", "status": "in_progress" },
                { "id": 3, "content": "next", "status": "pending" }
            ]}),
        )
        .await
        .expect("invoke");
        let rendered = tool.store.render().await;
        assert!(rendered.contains("[x] 1: done"));
        assert!(rendered.contains("[~] 2: busy"));
        assert!(rendered.contains("[ ] 3: next"));
    }
}
