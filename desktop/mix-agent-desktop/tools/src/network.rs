//! Network Tool: HTTP fetch with egress policy + optional DuckDuckGo search.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::egress::EgressPolicy;
use super::{Tool, ToolContext, ToolResult};

#[derive(Default)]
pub struct NetworkTool {
    pub egress: EgressPolicy,
}

impl NetworkTool {
    pub fn new(egress: EgressPolicy) -> Self {
        Self { egress }
    }
}

#[async_trait]
impl Tool for NetworkTool {
    fn id(&self) -> &str {
        "network"
    }
    fn name(&self) -> &str {
        "Network"
    }
    fn description(&self) -> &str {
        "GET / POST JSON / text with egress allowlist and private-IP deny."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Absolute http(s) URL" },
                "method": { "type": "string", "enum": ["GET", "POST", "PUT", "DELETE"], "default": "GET" },
                "headers": { "type": "object", "additionalProperties": { "type": "string" } },
                "body": { "description": "Optional JSON / string body" }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }
    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("url is required"))?;
        self.egress.enforce(url)?;
        let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
        let headers = args
            .get("headers")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let body = args.get("body").cloned();
        let client = reqwest::Client::builder()
            .user_agent("mix-agent-desktop/0.1")
            .build()?;
        let mut req = client.request(method.parse().unwrap_or(reqwest::Method::GET), url);
        if let Some(obj) = headers.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    req = req.header(k, s);
                }
            }
        }
        if let Some(body) = body {
            if body.is_string() {
                if let Some(s) = body.as_str() {
                    req = req.body(s.to_string());
                }
            } else {
                req = req.json(&body);
            }
        }
        let response = req.send().await?;
        let status = response.status().as_u16();
        let text = response.text().await?;
        Ok(ToolResult {
            ok: status < 400,
            content: serde_json::json!({ "status": status, "body": text }),
            error: if status >= 400 {
                Some(format!("HTTP {}", status))
            } else {
                None
            },
        })
    }
}

#[derive(Default)]
pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn id(&self) -> &str {
        "web_search"
    }
    /// A query against a fixed search engine. Note this is deliberately *not*
    /// shared with `web_fetch`: a model-chosen URL can exfiltrate whatever is
    /// in the prompt, and that one is gated.
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }
    fn name(&self) -> &str {
        "Web search"
    }
    fn description(&self) -> &str {
        "DuckDuckGo search and HTML extraction."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "max_results": { "type": "integer", "default": 6 }
            },
            "required": ["query"]
        })
    }
    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("query required"))?;
        let max = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(6);
        let ddgs = ddgs::Ddgs::new().map_err(|e| anyhow::anyhow!("ddgs init: {}", e))?;
        let results = ddgs
            .text(query)
            .await
            .map_err(|e| anyhow::anyhow!("ddgs search: {}", e))?
            .into_iter()
            .take(max as usize)
            .map(|r| serde_json::json!({ "title": r.title, "url": r.href, "snippet": r.body }))
            .collect::<Vec<_>>();
        Ok(ToolResult {
            ok: true,
            content: serde_json::json!(results),
            error: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}
