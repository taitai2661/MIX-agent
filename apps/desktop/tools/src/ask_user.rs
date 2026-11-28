//! Structured user question tool (S2 producer).
//!
//! The model calls `ask_user` when it needs a decision it cannot make alone.
//! The tool validates the card and returns it; the engine emits
//! `question_requested`, blocks for `answer_question` steering, then returns
//! the answer to the model as the tool result.

use super::{Tool, ToolContext, ToolResult};
use crate::approval::QuestionCard;

#[derive(Default)]
pub struct AskUserTool;

fn parse_options(args: &serde_json::Value) -> Result<Vec<crate::approval::QuestionOption>, String> {
    let arr = args
        .get("options")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "options is required (array of {label, description})".to_string())?;
    let mut out = Vec::new();
    for item in arr {
        if let Some(label) = item.as_str() {
            out.push(crate::approval::QuestionOption {
                label: label.to_string(),
                description: String::new(),
            });
        } else {
            let label = item
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let description = item
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            out.push(crate::approval::QuestionOption { label, description });
        }
    }
    Ok(out)
}

#[async_trait::async_trait]
impl Tool for AskUserTool {
    fn id(&self) -> &str {
        "ask_user"
    }
    fn name(&self) -> &str {
        "Ask user"
    }
    fn is_sensitive(&self, _args: &serde_json::Value) -> bool {
        false
    }
    fn description(&self) -> &str {
        "Ask the user a structured question mid-Run. Returns the card; the engine waits for the user's answer and returns it as the tool result."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": { "type": "string" },
                "options": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "description": { "type": "string" }
                        },
                        "required": ["label"]
                    }
                },
                "multi_select": { "type": "boolean", "default": false },
                "recommended": { "type": "string" }
            },
            "required": ["question", "options"]
        })
    }
    async fn invoke(
        &self,
        _ctx: ToolContext,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult> {
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let options = parse_options(&args).map_err(|e| anyhow::anyhow!(e))?;
        let multi_select = args
            .get("multi_select")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let recommended = args
            .get("recommended")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let card = QuestionCard {
            question,
            options,
            multi_select,
            recommended,
        };
        card.validate().map_err(|e| anyhow::anyhow!(e))?;
        let question_id = uuid::Uuid::new_v4().to_string();
        Ok(ToolResult::ok(serde_json::json!({
            "kind": "ask_user",
            "question_id": question_id,
            "question": card.question,
            "options": card.options,
            "multi_select": card.multi_select,
            "recommended": card.recommended,
        })))
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
            additional_paths: Vec::new(),
            approval_token: None,
        }
    }

    #[tokio::test]
    async fn validates_card_before_returning() {
        let tool = AskUserTool;
        let bad = tool
            .invoke(ctx(), serde_json::json!({"question": "", "options": []}))
            .await;
        assert!(bad.is_err());
        let ok = tool
            .invoke(
                ctx(),
                serde_json::json!({
                    "question": "どれを使いますか?",
                    "options": [{"label": "A"}, {"label": "B", "description": "second"}],
                    "recommended": "A"
                }),
            )
            .await
            .expect("valid card");
        assert!(ok.ok);
        assert_eq!(ok.content["question"], "どれを使いますか?");
    }
}
