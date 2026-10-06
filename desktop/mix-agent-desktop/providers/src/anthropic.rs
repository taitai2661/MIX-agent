//! Anthropic Messages API adapter. SSE events use `event:` / `data:` pairs and
//! yield `message_start`, `content_block_start`, `content_block_delta`,
//! `content_block_stop`, `message_delta`, `message_stop`.

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use super::transport;
use super::{
    Provider, ProviderCapabilities, ProviderId, ProviderRequest, ReasoningControl, StreamEvent,
    Usage,
};

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    pub base_url: String,
    pub api_key: Option<String>,
}

impl AnthropicProvider {
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
        }
    }
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    messages: &'a [AnthropicMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [AnthropicTool]>,
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
}

#[derive(Serialize)]
struct AnthropicThinking {
    #[serde(rename = "type")]
    kind: &'static str,
    budget_tokens: u32,
}

/// A message is a role plus a list of typed content blocks. Flattening this to a
/// single string is what previously broke tool use: `tool_use` has to travel
/// back to the API inside the assistant turn, and `tool_result` inside the user
/// turn, or the model cannot match a result to its call.
#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: Vec<AnthropicBlock>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "serde_json::Value::is_null")]
        is_error: serde_json::Value,
    },
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Deserialize, Default)]
struct AnthropicEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    index: i64,
    #[serde(default)]
    content_block: Option<AnthropicContentBlock>,
    #[serde(default)]
    delta: AnthropicDelta,
    #[serde(default)]
    message: Option<AnthropicMessageMeta>,
    /// Top-level `usage` of a `message_delta` / `message_start` event.
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize, Default)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    input: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code)]
    text: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    thinking: Option<String>,
}

#[derive(Deserialize, Default)]
struct AnthropicDelta {
    // A `message_delta` payload carries `{"stop_reason": …, "stop_sequence": …}`
    // with no `type` field at all; requiring it made the whole event fail to
    // deserialize, which is how the completion signal used to be lost.
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct AnthropicMessageMeta {
    #[serde(default)]
    #[allow(dead_code)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize, Default)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &ProviderId {
        &self.base_url
    }
    fn label(&self) -> &str {
        "Anthropic"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            vision: true,
            tool_calling: true,
            reasoning: true,
            streaming: true,
        }
    }

    async fn stream(&self, req: ProviderRequest) -> anyhow::Result<super::StreamBox> {
        let api_key = self
            .api_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Anthropic provider requires an API key"))?;
        let base_url = self.base_url.trim_end_matches('/').to_string();
        let url = format!("{}/v1/messages", base_url);

        let (inlined_system, messages) = build_messages(&req);
        let tools = if req.tools.is_empty() {
            None
        } else {
            Some(
                req.tools
                    .iter()
                    .map(|t| AnthropicTool {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        input_schema: t.parameters.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let mut thinking = match req.reasoning {
            Some(ReasoningControl::Low) => Some(AnthropicThinking {
                kind: "enabled",
                budget_tokens: 1024,
            }),
            Some(ReasoningControl::Medium) => Some(AnthropicThinking {
                kind: "enabled",
                budget_tokens: 4096,
            }),
            Some(ReasoningControl::High) => Some(AnthropicThinking {
                kind: "enabled",
                budget_tokens: 16384,
            }),
            Some(ReasoningControl::Xhigh) => Some(AnthropicThinking {
                kind: "enabled",
                budget_tokens: 32768,
            }),
            Some(ReasoningControl::Custom { budget_tokens }) => Some(AnthropicThinking {
                kind: "enabled",
                budget_tokens,
            }),
            _ => None,
        };
        let mut max_tokens = req.max_tokens.unwrap_or(4096);
        if let Some(t) = thinking.as_mut() {
            // The API rejects `budget_tokens >= max_tokens`, and the default
            // `max_tokens` of 4096 is exactly the Medium budget.
            if req.max_tokens.is_none() {
                max_tokens = t.budget_tokens.saturating_add(4096).min(64_000);
            }
            t.budget_tokens = t.budget_tokens.min(max_tokens.saturating_sub(1).max(1));
        }
        let body = AnthropicRequest {
            model: &req.model,
            messages: &messages,
            system: inlined_system.as_deref(),
            tools: tools.as_deref(),
            max_tokens,
            stream: true,
            temperature: req.temperature,
            thinking,
        };

        let client = transport::client();
        let request = client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body);
        // Anthropic rate limits aggressively on burst traffic; without the
        // retry a single 429 ended the whole run.
        let response = transport::send_with_retry(request).await?;

        let sse = transport::sse_stream(response);
        let buffer = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));

        Ok(Box::pin(an_sse_stream(buffer, sse)))
    }
}

fn an_sse_stream(
    buffer: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    sse: std::pin::Pin<Box<dyn Stream<Item = anyhow::Result<Bytes>> + Send>>,
) -> impl Stream<Item = StreamEvent> + Send + 'static {
    let mut sse = sse;
    async_stream::stream! {
        let mut tool_state: std::collections::HashMap<i64, (String, String, String)> = Default::default();
        // `message_start` carries the input/cache tokens, `message_delta` the
        // output tokens and the stop reason; neither alone is a complete Usage.
        let mut usage: Option<Usage> = None;
        let mut finished = false;
        while let Some(item) = sse.next().await {
            let bytes = match item {
                Ok(b) => b,
                Err(e) => {
                    yield StreamEvent::Error(format!("SSE read: {}", e));
                    return;
                }
            };
            let mut buf = buffer.lock().await;
            let events = match transport::parse_sse_lines(&mut buf, &bytes) {
                Ok(events) => events,
                Err(e) => {
                    drop(buf);
                    yield StreamEvent::Error(e);
                    return;
                }
            };
            drop(buf);
            for raw in events {
                let event: AnthropicEvent = match serde_json::from_str(&raw) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                match event.kind.as_str() {
                    "content_block_start" => {
                        if let Some(block) = event.content_block {
                            if block.kind == "tool_use" {
                                let id = block.id.clone().unwrap_or_default();
                                let name = block.name.clone().unwrap_or_default();
                                tool_state.insert(event.index, (id.clone(), name.clone(), String::new()));
                                if !id.is_empty() && !name.is_empty() {
                                    yield StreamEvent::ToolCallStart { id, name };
                                }
                            }
                        }
                    }
                    "content_block_delta" => {
                        match event.delta.kind.as_str() {
                            "text_delta" => {
                                if let Some(text) = event.delta.text {
                                    if !text.is_empty() {
                                        yield StreamEvent::TextDelta(text);
                                    }
                                }
                            }
                            "thinking_delta" => {
                                if let Some(text) = event.delta.thinking {
                                    if !text.is_empty() {
                                        yield StreamEvent::ReasoningDelta(text);
                                    }
                                }
                            }
                            "input_json_delta" => {
                                if let Some(delta) = event.delta.partial_json {
                                    if !delta.is_empty() {
                                        let entry = tool_state.entry(event.index).or_insert_with(|| (String::new(), String::new(), String::new()));
                                        entry.2.push_str(&delta);
                                        if !entry.0.is_empty() {
                                            yield StreamEvent::ToolCallArgumentDelta {
                                                id: entry.0.clone(),
                                                delta,
                                            };
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    "content_block_stop" => {
                        if let Some((id, name, args)) = tool_state.remove(&event.index) {
                            if !id.is_empty() {
                                yield StreamEvent::ToolCallFinish { id, name, arguments: args };
                            }
                        }
                    }
                    "message_start" => {
                        let start = event
                            .message
                            .and_then(|m| m.usage)
                            .or(event.usage);
                        if let Some(u) = start {
                            let cached = u.cache_read_input_tokens;
                            let written = u.cache_creation_input_tokens;
                            usage = Some(Usage {
                                input_tokens: u.input_tokens,
                                output_tokens: 0,
                                cache_read_tokens: cached,
                                cache_write_tokens: written,
                            });
                        }
                    }
                    "message_delta" => {
                        // Usage lives at the top level of `message_delta`, not
                        // under `delta` and not under a `message` object.
                        if let Some(u) = event.usage {
                            let entry = usage.get_or_insert(Usage {
                                input_tokens: 0,
                                output_tokens: 0,
                                cache_read_tokens: 0,
                                cache_write_tokens: 0,
                            });
                            entry.output_tokens = u.output_tokens;
                            if entry.input_tokens == 0 {
                                entry.input_tokens = u.input_tokens;
                            }
                            if entry.cache_read_tokens == 0 {
                                entry.cache_read_tokens = u.cache_read_input_tokens;
                            }
                            if entry.cache_write_tokens == 0 {
                                entry.cache_write_tokens = u.cache_creation_input_tokens;
                            }
                        }
                        if let Some(meta) = event.message {
                            if let Some(u) = meta.usage {
                                let entry = usage.get_or_insert(Usage {
                                    input_tokens: 0,
                                    output_tokens: 0,
                                    cache_read_tokens: 0,
                                    cache_write_tokens: 0,
                                });
                                entry.output_tokens = u.output_tokens;
                            }
                        }
                        if !finished {
                            finished = true;
                            yield StreamEvent::Finished {
                                finish_reason: event
                                    .delta
                                    .stop_reason
                                    .unwrap_or_else(|| "stop".into()),
                                usage: usage.clone(),
                            };
                        }
                    }
                    "message_stop" => {
                        // Safety net: some gateways end the stream without a
                        // usable `message_delta`, and the engine treats a stream
                        // that never finishes as a failed run.
                        if !finished {
                            yield StreamEvent::Finished {
                                finish_reason: "stop".into(),
                                usage: usage.clone(),
                            };
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }
        // The connection can also close without `message_stop`.
        if !finished {
            yield StreamEvent::Finished {
                finish_reason: "stop".into(),
                usage,
            };
        }
    }
}

fn build_messages(req: &ProviderRequest) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut out: Vec<AnthropicMessage> = Vec::new();
    let mut system_parts: Vec<String> = Vec::new();
    if let Some(system) = req.system.as_deref() {
        if !system.trim().is_empty() {
            system_parts.push(system.to_string());
        }
    }

    // Tool-use ids awaited by the most recent assistant turn. A `tool` message
    // is only kept when its id appears here: a `tool_result` with no matching
    // `tool_use` is rejected by the API, and that is exactly what a truncated
    // or resumed history produces.
    let mut awaited: Vec<String> = Vec::new();

    for m in &req.messages {
        match m.role.as_str() {
            // The Messages API has a single top-level `system` field, so every
            // system-role message the engine injects (`<task_state>`,
            // `<compressed_history>`, `<retrieved_context>`) is folded into it
            // rather than dropped.
            "system" => {
                if !m.content.trim().is_empty() {
                    system_parts.push(m.content.clone());
                }
            }
            "tool" => {
                let Some(call_id) = m.tool_call_id.as_deref() else {
                    continue;
                };
                if !awaited.iter().any(|id| id == call_id) {
                    continue;
                }
                let block = AnthropicBlock::ToolResult {
                    tool_use_id: call_id.to_string(),
                    content: m.content.clone(),
                    is_error: if m.is_error {
                        serde_json::Value::Bool(true)
                    } else {
                        serde_json::Value::Null
                    },
                };
                // Results for parallel tool calls belong in one user turn, so
                // append to the previous user message when that is what it is.
                match out.last_mut() {
                    Some(last) if last.role == "user" && last.content_is_tool_results() => {
                        last.content.push(block);
                    }
                    _ => out.push(AnthropicMessage {
                        role: "user",
                        content: vec![block],
                    }),
                }
            }
            "assistant" => {
                let mut blocks = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(AnthropicBlock::Text {
                        text: m.content.clone(),
                    });
                }
                for call in &m.tool_calls {
                    let input = serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    blocks.push(AnthropicBlock::ToolUse {
                        id: call.id.clone(),
                        name: call.function.name.clone(),
                        input,
                    });
                }
                if blocks.is_empty() {
                    continue;
                }
                awaited = m.tool_calls.iter().map(|c| c.id.clone()).collect();
                out.push(AnthropicMessage {
                    role: "assistant",
                    content: blocks,
                });
            }
            _ => {
                awaited.clear();
                if m.content.is_empty() {
                    continue;
                }
                out.push(AnthropicMessage {
                    role: "user",
                    content: vec![AnthropicBlock::Text {
                        text: m.content.clone(),
                    }],
                });
            }
        }
    }

    // The API requires the history to start with a user turn, which a resumed
    // checkpoint can violate. Trimming it can orphan the tool results that
    // answered it, so those go too.
    let mut drop = 0usize;
    while matches!(out.get(drop), Some(first) if first.role == "assistant") {
        drop += 1;
    }
    if drop > 0 {
        let surviving: Vec<String> = out[drop..]
            .iter()
            .flat_map(|m| match m.role {
                "assistant" => m.tool_use_ids(),
                _ => Vec::new(),
            })
            .collect();
        out.drain(..drop);
        out.retain(|m| {
            !(m.role == "user" && m.content_is_tool_results())
                || m.tool_result_ids().iter().all(|id| surviving.contains(id))
        });
    }
    out.retain(|m| !m.content.is_empty());

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };
    (system, out)
}

impl AnthropicMessage {
    /// True when every block in this turn is a tool result, which is the only
    /// kind of user turn that may be merged with a following one.
    fn content_is_tool_results(&self) -> bool {
        !self.content.is_empty()
            && self
                .content
                .iter()
                .all(|b| matches!(b, AnthropicBlock::ToolResult { .. }))
    }

    fn tool_use_ids(&self) -> Vec<String> {
        self.content
            .iter()
            .filter_map(|b| match b {
                AnthropicBlock::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    fn tool_result_ids(&self) -> Vec<String> {
        self.content
            .iter()
            .filter_map(|b| match b {
                AnthropicBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AssistantToolCall;
    use crate::AssistantToolCallFn;
    use crate::Message;

    fn feed(
        chunks: &[&str],
    ) -> std::pin::Pin<Box<dyn Stream<Item = anyhow::Result<Bytes>> + Send>> {
        let items = chunks
            .iter()
            .map(|c| Ok(Bytes::from(c.as_bytes().to_vec())))
            .collect::<Vec<_>>();
        Box::pin(futures_util::stream::iter(items))
    }

    async fn collect_sse(chunks: &[&str]) -> Vec<StreamEvent> {
        let buffer = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
        an_sse_stream(buffer, feed(chunks)).collect().await
    }

    fn finished(events: &[StreamEvent]) -> Vec<(String, Option<Usage>)> {
        events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Finished {
                    finish_reason,
                    usage,
                } => Some((finish_reason.clone(), usage.clone())),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_real_message_delta_finishes_the_stream_with_usage() {
        let events = collect_sse(&[
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":4}}}\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":34}}\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n",
        ])
        .await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::TextDelta(t) if t == "hello")),
            "the text delta must survive: {events:?}"
        );
        let done = finished(&events);
        assert_eq!(done.len(), 1, "exactly one completion signal: {done:?}");
        assert_eq!(done[0].0, "end_turn");
        let usage = done[0].1.clone().expect("usage must be reported");
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 34);
        assert_eq!(usage.cache_read_tokens, 4);
    }

    #[tokio::test]
    async fn a_stream_that_ends_without_message_stop_still_finishes() {
        let events = collect_sse(&[
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n",
        ])
        .await;
        assert_eq!(finished(&events).len(), 1, "the engine needs a Finished");
    }

    fn request(messages: Vec<Message>) -> ProviderRequest {
        ProviderRequest {
            model: "claude".into(),
            messages,
            system: None,
            tools: std::sync::Arc::new(Vec::new()),
            temperature: None,
            top_p: None,
            max_tokens: None,
            reasoning: None,
            stream: true,
        }
    }

    fn tool_call(id: &str, name: &str) -> AssistantToolCall {
        AssistantToolCall {
            id: id.into(),
            kind: "function".into(),
            function: AssistantToolCallFn {
                name: name.into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            },
        }
    }

    /// The shape the engine always produces: a user turn, the assistant's tool
    /// call, then the results.
    fn tool_turn(calls: Vec<AssistantToolCall>) -> Vec<Message> {
        let results: Vec<Message> = calls
            .iter()
            .map(|c| Message::tool_result(c, format!("result for {}", c.function.name), false))
            .collect();
        let mut out = vec![Message::text("user", "edit a.rs")];
        out.push(Message::assistant_with_tools("", calls));
        out.extend(results);
        out
    }

    #[test]
    fn assistant_tool_use_travels_back_to_the_api() {
        let (_, messages) = build_messages(&request(tool_turn(vec![tool_call("call_1", "edit")])));
        let assistant = messages
            .iter()
            .find(|m| m.role == "assistant")
            .expect("assistant turn");
        match assistant.content.iter().find_map(|b| match b {
            AnthropicBlock::ToolUse { id, name, input } => Some((id, name, input)),
            _ => None,
        }) {
            Some((id, name, input)) => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "edit");
                assert_eq!(input["path"], "a.rs");
            }
            None => panic!("no tool_use block in {assistant:?}"),
        }
    }

    #[test]
    fn tool_results_become_a_user_turn() {
        let call = tool_call("call_1", "edit");
        let (_, messages) = build_messages(&request(tool_turn(vec![call])));
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].role, "user");
        match &messages[2].content[0] {
            AnthropicBlock::ToolResult { tool_use_id, .. } => assert_eq!(tool_use_id, "call_1"),
            other => panic!("expected tool_result, got {other:?}"),
        }
    }

    #[test]
    fn parallel_tool_results_are_merged_into_one_user_turn() {
        let (_, messages) = build_messages(&request(tool_turn(vec![
            tool_call("a", "grep"),
            tool_call("b", "glob"),
        ])));
        assert_eq!(messages.len(), 3, "results must not become two user turns");
        assert_eq!(messages[2].content.len(), 2);
    }

    #[test]
    fn a_failed_tool_result_sets_is_error() {
        let call = tool_call("a", "edit");
        let (_, messages) = build_messages(&request(vec![
            Message::text("user", "go"),
            Message::assistant_with_tools("", vec![call.clone()]),
            Message::tool_result(&call, "old_string was not found", true),
        ]));
        match &messages[2].content[0] {
            AnthropicBlock::ToolResult { is_error, .. } => {
                assert_eq!(*is_error, serde_json::Value::Bool(true));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_successful_tool_result_omits_is_error() {
        let call = tool_call("a", "edit");
        let (_, messages) = build_messages(&request(vec![
            Message::text("user", "go"),
            Message::assistant_with_tools("", vec![call.clone()]),
            Message::tool_result(&call, "done", false),
        ]));
        match &messages[2].content[0] {
            AnthropicBlock::ToolResult { is_error, .. } => {
                assert!(is_error.is_null(), "is_error must be omitted, not false");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn system_messages_are_folded_into_the_system_field() {
        let (system, messages) = build_messages(&request(vec![
            Message::text("system", "<task_state>goal</task_state>"),
            Message::text("user", "hi"),
        ]));
        assert!(system.expect("system text").contains("<task_state>"));
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn a_leading_assistant_turn_is_dropped() {
        // The API requires the history to start with a user turn, which is what
        // a resumed checkpoint can violate.
        let (_, messages) = build_messages(&request(vec![
            Message::text("assistant", "starting"),
            Message::text("user", "hi"),
        ]));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
    }

    #[test]
    fn a_trailing_orphan_tool_result_is_dropped() {
        let (_, messages) = build_messages(&request(vec![
            Message::text("user", "hi"),
            Message {
                role: "tool".into(),
                content: "orphan".into(),
                tool_calls: Vec::new(),
                tool_call_id: Some("missing".into()),
                name: Some("edit".into()),
                is_error: false,
            },
        ]));
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn a_result_whose_call_was_trimmed_is_dropped() {
        // Trimming the leading assistant turn orphans its result, so the result
        // has to go too rather than reference a tool_use that is no longer there.
        let call = tool_call("a", "edit");
        let (_, messages) = build_messages(&request(vec![
            Message::assistant_with_tools("", vec![call.clone()]),
            Message::tool_result(&call, "orphan", false),
        ]));
        assert!(messages.is_empty(), "{messages:?}");
    }

    #[test]
    fn a_result_after_a_new_user_turn_is_dropped() {
        let call = tool_call("a", "edit");
        let (_, messages) = build_messages(&request(vec![
            Message::text("user", "go"),
            Message::assistant_with_tools("", vec![call.clone()]),
            Message::text("user", "actually, never mind"),
            Message::tool_result(&call, "too late", false),
        ]));
        assert_eq!(messages.len(), 3);
        assert!(!messages.iter().any(|m| m.content_is_tool_results()));
    }

    #[test]
    fn empty_messages_are_skipped() {
        let (_, messages) = build_messages(&request(vec![
            Message::text("user", ""),
            Message::text("user", "real"),
        ]));
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn serialised_tool_use_has_the_wire_shape() {
        let (_, messages) = build_messages(&request(tool_turn(vec![tool_call("call_1", "edit")])));
        let assistant = messages
            .iter()
            .find(|m| m.role == "assistant")
            .expect("assistant turn");
        let json = serde_json::to_value(assistant).expect("serialize");
        let block = json["content"]
            .as_array()
            .expect("blocks")
            .iter()
            .find(|b| b["type"] == "tool_use")
            .expect("tool_use block");
        assert_eq!(block["id"], "call_1");
        assert_eq!(block["name"], "edit");
        assert_eq!(block["input"]["path"], "a.rs");
    }

    #[test]
    fn serialised_tool_result_has_the_wire_shape() {
        let call = tool_call("call_1", "edit");
        let (_, messages) = build_messages(&request(vec![
            Message::text("user", "go"),
            Message::assistant_with_tools("", vec![call.clone()]),
            Message::tool_result(&call, "boom", true),
        ]));
        let json = serde_json::to_value(&messages[2]).expect("serialize");
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"][0]["type"], "tool_result");
        assert_eq!(json["content"][0]["tool_use_id"], "call_1");
        assert_eq!(json["content"][0]["is_error"], true);
    }
}
