//! Generic Web Tool: HTTP fetch with structured HTML extraction.

use async_trait::async_trait;

use super::network::NetworkTool;
use super::{Tool, ToolContext, ToolResult};

pub struct WebFetchTool(pub NetworkTool);

impl WebFetchTool {
    pub fn new(network: NetworkTool) -> Self {
        Self(network)
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn id(&self) -> &str {
        "web_fetch"
    }
    fn name(&self) -> &str {
        "Web Fetch"
    }
    fn description(&self) -> &str {
        "GET a URL and return JSON / text body."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" }
            },
            "required": ["url"]
        })
    }
    async fn invoke(
        &self,
        ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        self.0.invoke(ctx, args).await
    }
}
