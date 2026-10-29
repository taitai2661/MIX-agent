//! User-facing Plan Tool: renders a structured proposal in the transcript.
//!
//! The Tool is sensitive, so the call goes through the ordinary approval
//! prompt: the Run pauses until the user approves or denies the plan. Approving
//! it is what lifts Plan mode inside the engine for the rest of the Run (see
//! `RunEngine`), so "present a plan" and "start implementing it" are one
//! continuous flow instead of two conversations.

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
    /// Presenting the plan *is* the approval gate: approving the prompt lifts
    /// Plan mode for the rest of the Run.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Present a structured plan to the user for review. The Run pauses until the \
         user approves or denies it; approval lifts Plan mode for this Run."
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
        let title = crate::args::str_or(&args, "title", "Plan").to_string();
        let steps = crate::args::str_vec(&args, "steps");
        let risk = crate::args::str_or(&args, "risk", "low").to_string();
        let approval_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        Ok(ToolResult::ok(serde_json::json!({
            "kind": "user_plan",
            "approval_id": approval_id,
            "title": title,
            "steps": steps,
            "risk": risk,
            "expires_at": (now + Duration::hours(1)).to_rfc3339(),
        })))
    }
}
