//! The Browser Tool contract surfaced to the Tool Registry.
//!
//! Concrete backends live behind [`crate::operator::BrowserOperator`], so the
//! Agent never depends on `chromiumoxide` directly.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCall {
    pub workspace_id: uuid::Uuid,
    pub action: BrowserAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserAction {
    Navigate {
        url: String,
    },
    Click {
        selector: String,
    },
    Type {
        selector: String,
        text: String,
    },
    Scroll {
        x: i32,
        y: i32,
    },
    Dom {
        selector: Option<String>,
    },
    Screenshot {
        full_page: bool,
    },
    Evaluate {
        script: String,
    },
    WaitFor {
        condition: WaitCondition,
        timeout_ms: u64,
    },
    /// Read the current URL and title.
    Info,
    Close,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitCondition {
    Selector { selector: String },
    Url { url: String },
    Function { script: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResponse {
    pub ok: bool,
    pub payload: serde_json::Value,
    pub artifact_id: Option<String>,
    pub error: Option<String>,
}

impl BrowserResponse {
    pub fn ok(payload: serde_json::Value) -> Self {
        Self {
            ok: true,
            payload,
            artifact_id: None,
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            ok: false,
            payload: serde_json::Value::Null,
            artifact_id: None,
            error: Some(message),
        }
    }
}

#[async_trait]
pub trait BrowserTool: Send + Sync {
    async fn invoke(&self, call: BrowserCall) -> anyhow::Result<BrowserResponse>;
}

/// JSON schema for the single `browser` Tool the Agent sees.
pub fn browser_tool_spec() -> serde_json::Value {
    serde_json::json!({
        "name": "browser",
        "description": "Drive a real Chromium instance: navigate, click, type, scroll, read the DOM, screenshot, evaluate JavaScript, and wait for conditions. Each workspace keeps its own persistent profile so logins survive between runs.",
        "parameters": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "click", "type", "scroll", "dom", "screenshot", "evaluate", "wait_for", "info", "close"],
                    "description": "Operation to perform."
                },
                "url": { "type": "string", "description": "Target URL for navigate / wait_for(url)." },
                "selector": { "type": "string", "description": "CSS selector for click / type / dom / wait_for(selector)." },
                "text": { "type": "string", "description": "Text to type." },
                "x": { "type": "integer", "description": "Scroll delta X." },
                "y": { "type": "integer", "description": "Scroll delta Y." },
                "script": { "type": "string", "description": "JavaScript for evaluate / wait_for(function)." },
                "full_page": { "type": "boolean", "default": false, "description": "Capture the full scrollable page." },
                "timeout_ms": { "type": "integer", "default": 10000 }
            },
            "required": ["action"]
        }
    })
}
