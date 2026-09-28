//! User-facing Plan Tool: blocks the Run until the user confirms the agent's
//! structured proposal.

use async_trait::async_trait;
use chrono::{Duration, Utc};

use super::{Tool, ToolContext, ToolResult};

#[derive(Default)]
pub struct UserPlanTool;

#[async_trait]
impl Tool for UserPlanTool {
    fn id(&self) -> &str {
        "user_plan"
    }
    fn name(&self) -> &str {
        "User plan"
    }
    fn description(&self) -> &str {
        "Ask the user to confirm a structured plan before proceeding."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" },
                "steps": { "type": "array", "items": { "type": "string" } },
                "risk": { "type": "string", "enum": ["low", "medium", "high"], "default": "low" }
            },
            "required": ["title"]
        })
    }
    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Plan")
            .to_string();
        let steps: Vec<String> = args
            .get("steps")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let risk = args
            .get("risk")
            .and_then(|v| v.as_str())
            .unwrap_or("low")
            .to_string();
        let approval_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        Ok(ToolResult {
            ok: true,
            content: serde_json::json!({
                "kind": "user_plan",
                "approval_id": approval_id,
                "title": title,
                "steps": steps,
                "risk": risk,
                "expires_at": (now + Duration::hours(1)).to_rfc3339(),
            }),
            error: None,
        })
    }
}
