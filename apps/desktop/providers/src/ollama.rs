//! Native Ollama chat transport.
//!
//! Ollama's OpenAI-compatible endpoint is enough for plain chat, but it cannot
//! express the two settings that matter most to a local model: the context
//! window (`num_ctx`) and how long the model stays resident (`keep_alive`).
//! This adapter talks to `/api/chat` directly so the engine can pass both.
//!
//! Embeddings stay on the OpenAI-compatible `/v1/embeddings` endpoint via the
//! wrapped [`OpenAiProvider`], where they work identically and require no
//! second implementation.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Map, Value};

use crate::discovery::origin_of;
use crate::openai::OpenAiProvider;
use crate::{
    resolved_history_indices, Provider, ProviderCapabilities, ProviderId, ProviderRequest,
    ReasoningControl, StreamBox, StreamEvent, Usage,
};

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";

pub struct OllamaProvider {
    base_url: String,
    embedder: OpenAiProvider,
}

pub fn ollama(base_url: impl Into<String>) -> OllamaProvider {
    let base_url = base_url.into();
    OllamaProvider {
        embedder: OpenAiProvider::new(base_url.clone(), None),
        base_url,
    }
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        ollama(base_url)
    }
}

/// Convert the shared history into Ollama's message shape.
///
/// `/api/chat` accepts `tool_name` on tool messages and an object (not a JSON
/// string) for each tool-call argument. Both are easy to get wrong: sending the
/// OpenAI string form makes Ollama reject the request, and dropping `tool_name`
/// loses which result belongs to which call.
fn ollama_messages(req: &ProviderRequest) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(system) = req.system.as_deref().filter(|value| !value.is_empty()) {
        out.push(json!({ "role": "system", "content": system }));
    }
    for index in resolved_history_indices(&req.messages) {
        let message = &req.messages[index];
        let mut entry = Map::new();
        entry.insert("role".into(), Value::String(message.role.clone()));
        entry.insert("content".into(), Value::String(message.content.clone()));
        if !message.tool_calls.is_empty() {
            entry.insert(
                "tool_calls".into(),
                Value::Array(message.tool_calls.iter().map(tool_call_json).collect()),
            );
        }
        if message.role == "tool" {
            if let Some(name) = message.name.as_deref() {
                entry.insert("tool_name".into(), Value::String(name.to_string()));
            }
        }
        out.push(Value::Object(entry));
    }
    out
}

fn tool_call_json(call: &crate::AssistantToolCall) -> Value {
    let arguments = serde_json::from_str(&call.function.arguments)
        .unwrap_or_else(|_| Value::Object(Map::new()));
    json!({
        "function": {
            "name": call.function.name,
            "arguments": arguments,
        }
    })
}

fn request_body(req: &ProviderRequest) -> Value {
    let mut options = Map::new();
    if let Some(temperature) = req.temperature {
        options.insert("temperature".into(), json!(temperature));
    }
    if let Some(top_p) = req.top_p {
        options.insert("top_p".into(), json!(top_p));
    }
    if let Some(max_tokens) = req.max_tokens {
        options.insert("num_predict".into(), json!(max_tokens));
    }
    if let Some(context_window) = req.context_window.filter(|value| *value > 0) {
        options.insert("num_ctx".into(), json!(context_window));
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("messages".into(), Value::Array(ollama_messages(req)));
    body.insert("stream".into(), json!(true));
    if !options.is_empty() {
        body.insert("options".into(), Value::Object(options));
    }
    if let Some(keep_alive) = req.keep_alive.as_deref().filter(|value| !value.is_empty()) {
        body.insert("keep_alive".into(), json!(keep_alive));
    }
    if let Some(control) = req.reasoning {
        if !matches!(control, ReasoningControl::Off) {
            body.insert("think".into(), json!(true));
        }
    }
    if !req.tools.is_empty() {
        body.insert(
            "tools".into(),
            Value::Array(
                req.tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": tool.name,
                                "description": tool.description,
                                "parameters": tool.parameters,
                            }
                        })
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(body)
}

/// Whether this request asks the model to emit a thinking stream.
fn thinking_requested(req: &ProviderRequest) -> bool {
    matches!(
        req.reasoning,
        Some(control) if !matches!(control, ReasoningControl::Off)
    )
}

/// Whether an Ollama error means "this model has no thinking mode".
///
/// Ollama phrases this differently across versions ("… does not support
/// thinking", "thinking is not supported"), and the message arrives wrapped in
/// the transport's status text, so match on the intent rather than one string.
fn thinking_unsupported(err: &anyhow::Error) -> bool {
    let text = err.to_string().to_ascii_lowercase();
    text.contains("think") && (text.contains("not support") || text.contains("unsupported"))
}

async fn post_chat(url: &str, body: &Value) -> anyhow::Result<reqwest::Response> {
    let request = crate::transport::json_post(crate::transport::client(), url, body);
    crate::transport::send_with_retry(request).await
}

/// Turn one NDJSON line into zero or more stream events.
///
/// Split out from the async stream so the terminal `done` object and a line
/// with no trailing newline are handled by the same code path.
fn parse_line(line: &str, saw_tool_call: &mut bool) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return events,
    };
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        events.push(StreamEvent::Error(error.to_string()));
        return events;
    }
    if let Some(message) = value.get("message") {
        if let Some(thinking) = message.get("thinking").and_then(Value::as_str) {
            if !thinking.is_empty() {
                events.push(StreamEvent::ReasoningDelta(thinking.to_string()));
            }
        }
        if let Some(content) = message.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                events.push(StreamEvent::TextDelta(content.to_string()));
            }
        }
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let name = call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let arguments = call
                    .pointer("/function/arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let arguments = serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".into());
                // No de-duplication by name+arguments: a model is allowed to
                // call the same tool twice with identical input in one turn,
                // and dropping the second call silently loses work. Ollama
                // sends each call once, so there is nothing to collapse here.
                let id = format!("ollama_{}", uuid::Uuid::new_v4());
                *saw_tool_call = true;
                events.push(StreamEvent::ToolCallStart {
                    id: id.clone(),
                    name: name.clone(),
                });
                events.push(StreamEvent::ToolCallArgumentDelta {
                    id: id.clone(),
                    delta: arguments.clone(),
                });
                events.push(StreamEvent::ToolCallFinish {
                    id,
                    name,
                    arguments,
                });
            }
        }
    }
    if value.get("done").and_then(Value::as_bool).unwrap_or(false) {
        let usage = Usage {
            input_tokens: value
                .get("prompt_eval_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output_tokens: value.get("eval_count").and_then(Value::as_u64).unwrap_or(0),
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        let finish_reason = if *saw_tool_call {
            "tool_calls".to_string()
        } else {
            value
                .get("done_reason")
                .and_then(Value::as_str)
                .unwrap_or("stop")
                .to_string()
        };
        events.push(StreamEvent::Finished {
            finish_reason,
            usage: Some(usage),
        });
    }
    events
}

#[async_trait]
impl Provider for OllamaProvider {
    fn id(&self) -> &ProviderId {
        &self.base_url
    }

    fn label(&self) -> &str {
        "Ollama"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: true,
            tool_calling: true,
            reasoning: true,
            streaming: true,
        }
    }

    async fn stream(&self, req: ProviderRequest) -> anyhow::Result<StreamBox> {
        let url = format!("{}/api/chat", origin_of(&self.base_url));
        let body = request_body(&req);
        let response = match post_chat(&url, &body).await {
            Ok(response) => response,
            // Ollama rejects `think: true` for a model without a thinking
            // mode ("… does not support thinking"). The user asked for a
            // capability that this model lacks, not for a failed turn, so
            // retry once with thinking off instead of surfacing a 400.
            Err(err) if thinking_requested(&req) && thinking_unsupported(&err) => {
                let mut fallback = body;
                if let Some(object) = fallback.as_object_mut() {
                    object.remove("think");
                }
                post_chat(&url, &fallback).await?
            }
            Err(err) => return Err(err),
        };
        let mut stream = response.bytes_stream();
        Ok(Box::pin(async_stream::stream! {
            let mut buffer: Vec<u8> = Vec::new();
            let mut saw_tool_call = false;
            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        yield StreamEvent::Error(format!("Ollama stream failed: {err}"));
                        return;
                    }
                };
                buffer.extend_from_slice(&chunk);
                while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=position).collect();
                    let line = String::from_utf8_lossy(&line);
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    for event in parse_line(line, &mut saw_tool_call) {
                        let finished = matches!(event, StreamEvent::Finished { .. });
                        let failed = matches!(event, StreamEvent::Error(_));
                        yield event;
                        if finished || failed {
                            return;
                        }
                    }
                }
            }
            // Ollama does not guarantee a trailing newline. Process the final
            // NDJSON object before deciding the stream ended.
            let tail = String::from_utf8_lossy(&buffer);
            let tail = tail.trim();
            if !tail.is_empty() {
                for event in parse_line(tail, &mut saw_tool_call) {
                    let finished = matches!(event, StreamEvent::Finished { .. });
                    let failed = matches!(event, StreamEvent::Error(_));
                    yield event;
                    if finished || failed {
                        return;
                    }
                }
            }
            yield StreamEvent::Finished {
                finish_reason: if saw_tool_call { "tool_calls" } else { "stop" }.to_string(),
                usage: None,
            };
        }))
    }

    async fn embed(&self, model: &str, texts: &[String]) -> anyhow::Result<Vec<Vec<f64>>> {
        self.embedder.embed(model, texts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::{AssistantToolCall, AssistantToolCallFn, Message, ToolSpec};

    fn request(messages: Vec<Message>) -> ProviderRequest {
        ProviderRequest {
            model: "qwen3:8b".into(),
            messages,
            system: Some("be useful".into()),
            tools: Arc::new(Vec::new()),
            temperature: Some(0.2),
            top_p: None,
            max_tokens: Some(128),
            reasoning: None,
            stream: true,
            context_window: Some(8192),
            keep_alive: Some("5m".into()),
        }
    }

    #[test]
    fn thinking_errors_are_recognised() {
        assert!(thinking_unsupported(&anyhow::anyhow!(
            "provider HTTP 400: model does not support thinking"
        )));
        assert!(thinking_unsupported(&anyhow::anyhow!(
            "HTTP 400: thinking is not supported"
        )));
        assert!(!thinking_unsupported(&anyhow::anyhow!(
            "provider HTTP 500: internal error"
        )));
    }

    #[test]
    fn identical_tool_calls_are_not_deduplicated() {
        let mut saw_tool_call = false;
        let line = concat!(
            r#"{"message":{"tool_calls":["#,
            r#"{"function":{"name":"read","arguments":{"path":"a"}}},"#,
            r#"{"function":{"name":"read","arguments":{"path":"a"}}}]}}"#,
        );
        let events = parse_line(line, &mut saw_tool_call);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, StreamEvent::ToolCallStart { .. }))
                .count(),
            2,
            "a model may call the same tool twice with the same input"
        );
    }

    #[test]
    fn request_body_carries_num_ctx_and_keep_alive() {
        let body = request_body(&request(vec![Message::text("user", "hi")]));
        assert_eq!(body["options"]["num_ctx"], 8192);
        assert_eq!(body["options"]["num_predict"], 128);
        assert_eq!(body["keep_alive"], "5m");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hi");
    }

    #[test]
    fn tool_results_carry_their_tool_name() {
        let call = AssistantToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: AssistantToolCallFn {
                name: "read".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            },
        };
        let body = request_body(&ProviderRequest {
            tools: Arc::new(vec![ToolSpec {
                name: "read".into(),
                description: "read a file".into(),
                parameters: json!({"type":"object"}),
            }]),
            context_window: None,
            keep_alive: None,
            ..request(vec![
                Message::assistant_with_tools(String::new(), vec![call.clone()]),
                Message::tool_result(&call, "contents", false),
            ])
        });
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["function"]["name"],
            "read"
        );
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["function"]["arguments"]["path"],
            "a.rs"
        );
        assert_eq!(body["messages"][2]["tool_name"], "read");
        assert!(body
            .get("options")
            .and_then(|value| value.get("num_ctx"))
            .is_none());
        assert!(body.get("keep_alive").is_none());
    }
}
