//! Responses API request/stream conversion for ChatGPT / Codex.
//!
//! This module owns the wire shape only. Authentication, headers and quota
//! caching stay in the parent module, so the request builder can be unit
//! tested without a secret store or a network.

use std::collections::HashMap;
use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::{json, Value};

use super::ChatGptSession;
use crate::{transport, ProviderRequest, ReasoningControl, StreamBox, StreamEvent, Usage};

/// Build the Responses request body for one turn.
pub fn request_body(req: &ProviderRequest) -> Value {
    let mut instructions = req.system.clone().unwrap_or_default();
    let mut input: Vec<Value> = Vec::new();
    // The engine prepends a system message with the task state / todo list, so
    // overwriting `instructions` here would drop the actual system prompt (tool
    // policy, safety rules, …) whenever that head exists. Responses has a
    // single `instructions` string, so fold every system message into it.
    for index in crate::resolved_history_indices(&req.messages) {
        let message = &req.messages[index];
        match message.role.as_str() {
            "system" => {
                if !message.content.is_empty() {
                    if !instructions.is_empty() {
                        instructions.push_str("\n\n");
                    }
                    instructions.push_str(&message.content);
                }
            }
            "tool" => input.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id.clone().unwrap_or_default(),
                "output": message.content,
            })),
            _ if !message.native.is_empty() => input.extend(message.native.iter().cloned()),
            "assistant" => {
                if !message.content.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": message.content}],
                    }));
                }
                for call in &message.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.function.name,
                        "arguments": call.function.arguments,
                    }));
                }
            }
            role => input.push(json!({
                "role": role,
                "content": [{"type": "input_text", "text": message.content}],
            })),
        }
    }
    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect();
    // `Off` means "do not ask for reasoning". Mapping it to medium (the old
    // catch-all) silently overrode the user's choice. `Xhigh` is a real Codex
    // effort level; the OpenAI adapter already sends it, so pass it through.
    let effort = match req.reasoning {
        Some(ReasoningControl::Low) => Some("low"),
        Some(ReasoningControl::Medium) => Some("medium"),
        Some(ReasoningControl::High) => Some("high"),
        Some(ReasoningControl::Xhigh) => Some("xhigh"),
        Some(ReasoningControl::Auto) | None => Some("medium"),
        Some(ReasoningControl::Off) | Some(ReasoningControl::Custom { .. }) => None,
    };
    let cache_key = prompt_cache_key(req);
    let mut body = json!({
        "model": req.model,
        "input": input,
        "instructions": instructions,
        "tools": tools,
        "tool_choice": "auto",
        "parallel_tool_calls": true,
        "store": false,
        "stream": true,
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": req.max_tokens.unwrap_or(8192),
        "prompt_cache_key": cache_key,
    });
    if let Some(effort) = effort {
        body["reasoning"] = json!({"effort": effort, "summary": "auto"});
    }
    if req.tools.is_empty() {
        if let Some(object) = body.as_object_mut() {
            object.remove("tools");
            object.remove("tool_choice");
            object.remove("parallel_tool_calls");
        }
    }
    body
}

/// Stable cache key for one conversation.
///
/// The conversation id is the natural key and survives context compaction.
/// When a caller has no session (memory extraction, tool probe), fall back to
/// a deterministic FNV-1a hash instead of `DefaultHasher`, whose algorithm is
/// explicitly not stable across Rust releases.
fn prompt_cache_key(req: &ProviderRequest) -> String {
    if let Some(id) = req
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return format!("mix-{id}");
    }
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(req.model.as_bytes());
    feed(req.system.as_deref().unwrap_or_default().as_bytes());
    if let Some(first) = req.messages.iter().find(|m| m.role == "user") {
        feed(first.content.as_bytes());
    }
    format!("mix-{hash:016x}")
}

pub(super) fn truncate(body: &str, max: usize) -> String {
    if body.len() <= max {
        return body.to_string();
    }
    let mut cut = max;
    while cut > 0 && !body.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &body[..cut])
}

fn usage_from_value(value: &Value) -> Option<Usage> {
    let usage = value.get("response")?.get("usage")?;
    let input = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached = usage
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cached,
        cache_write_tokens: 0,
        context_tokens: Some(input),
    })
}

pub(super) fn event_stream(response: reqwest::Response, session: Arc<ChatGptSession>) -> StreamBox {
    Box::pin(cg_sse_stream(transport::sse_stream(response), session))
}

/// Convert the Codex SSE byte stream into [`StreamEvent`]s.
///
/// Kept separate from [`event_stream`] so tests can feed synthetic chunks
/// without a network or a `reqwest::Response` (the same pattern as the
/// Anthropic and Gemini adapters).
fn cg_sse_stream(
    mut sse: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = anyhow::Result<bytes::Bytes>> + Send>,
    >,
    session: Arc<ChatGptSession>,
) -> impl futures_util::Stream<Item = StreamEvent> + Send + 'static {
    async_stream::stream! {
        let mut buffer = Vec::<u8>::new();
        let mut item_calls: HashMap<String, String> = HashMap::new();
        let mut arguments: HashMap<String, String> = HashMap::new();
        while let Some(chunk) = sse.next().await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(err) => {
                    yield StreamEvent::Error(format!("SSE read: {err}"));
                    return;
                }
            };
            let events = match transport::parse_sse_lines(&mut buffer, &bytes) {
                Ok(events) => events,
                Err(err) => {
                    yield StreamEvent::Error(err);
                    return;
                }
            };
            for raw in events {
                if transport::sse_done(&raw) {
                    yield StreamEvent::Finished {
                        finish_reason: "stop".to_string(),
                        usage: None,
                    };
                    return;
                }
                let value: Value = match serde_json::from_str(&raw) {
                    Ok(value) => value,
                    Err(err) => {
                        yield StreamEvent::Error(format!("SSE JSON: {err}"));
                        continue;
                    }
                };
                let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
                match kind {
                    "response.output_text.delta" => {
                        if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                            if !delta.is_empty() {
                                yield StreamEvent::TextDelta(delta.to_string());
                            }
                        }
                    }
                    "response.reasoning_summary_text.delta" => {
                        if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                            if !delta.is_empty() {
                                yield StreamEvent::ReasoningDelta(delta.to_string());
                            }
                        }
                    }
                    "response.output_item.added" => {
                        let item = value.get("item").unwrap_or(&Value::Null);
                        if item.get("type").and_then(Value::as_str) == Some("function_call") {
                            let call_id = item
                                .get("call_id")
                                .or_else(|| item.get("id"))
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            let item_id = item
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            let name = item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            if !item_id.is_empty() && !call_id.is_empty() {
                                item_calls.insert(item_id, call_id.clone());
                            }
                            yield StreamEvent::ToolCallStart { id: call_id, name };
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        let item_id = value
                            .get("item_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let call_id = item_calls
                            .get(item_id)
                            .cloned()
                            .unwrap_or_else(|| item_id.to_string());
                        let delta = value
                            .get("delta")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if !delta.is_empty() {
                            arguments
                                .entry(call_id.clone())
                                .or_default()
                                .push_str(&delta);
                            yield StreamEvent::ToolCallArgumentDelta { id: call_id, delta };
                        }
                    }
                    "response.output_item.done" => {
                        let item = value.get("item").unwrap_or(&Value::Null);
                        if item.get("type").and_then(Value::as_str) == Some("function_call") {
                            let call_id = item
                                .get("call_id")
                                .or_else(|| item.get("id"))
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            let name = item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            let args = item
                                .get("arguments")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .or_else(|| arguments.get(&call_id).cloned())
                                .unwrap_or_default();
                            yield StreamEvent::ToolCallFinish {
                                id: call_id,
                                name,
                                arguments: args,
                            };
                        }
                    }
                    "response.completed" => {
                        let output = value
                            .pointer("/response/output")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        let finish_reason = if output
                            .iter()
                            .any(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
                        {
                            "tool_calls"
                        } else {
                            "stop"
                        };
                        yield StreamEvent::NativeOutput(output);
                        yield StreamEvent::Finished {
                            finish_reason: finish_reason.to_string(),
                            usage: usage_from_value(&value),
                        };
                        return;
                    }
                    "codex.rate_limits" => session.observe_rate_limit(&value),
                    "response.failed" | "response.incomplete" | "error" => {
                        let message = value
                            .pointer("/response/error/message")
                            .or_else(|| value.get("message"))
                            .and_then(Value::as_str)
                            .unwrap_or("provider stopped before completing the response");
                        yield StreamEvent::Error(message.to_string());
                        return;
                    }
                    _ => {}
                }
            }
        }
        yield StreamEvent::Error(
            "the provider stream ended without a completion signal".to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::StreamExt;

    fn feed(
        chunks: &[&str],
    ) -> std::pin::Pin<Box<dyn futures_util::Stream<Item = anyhow::Result<Bytes>> + Send>> {
        let items = chunks
            .iter()
            .map(|chunk| Ok(Bytes::from(chunk.as_bytes().to_vec())))
            .collect::<Vec<_>>();
        Box::pin(futures_util::stream::iter(items))
    }

    async fn collect(chunks: &[&str]) -> Vec<StreamEvent> {
        let session = ChatGptSession::new(
            "test",
            std::sync::Arc::new(crate::chatgpt::NoopCredentialStore),
            None,
        );
        cg_sse_stream(feed(chunks), session).collect().await
    }

    #[tokio::test]
    async fn text_deltas_and_completion_are_converted() {
        let events = collect(&[
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"message\"}],\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}}\n\n",
        ])
        .await;
        assert!(matches!(&events[0], StreamEvent::TextDelta(text) if text == "Hel"));
        assert!(matches!(&events[1], StreamEvent::TextDelta(text) if text == "lo"));
        assert!(matches!(
            &events[2],
            StreamEvent::NativeOutput(items) if items.len() == 1
        ));
        match &events[3] {
            StreamEvent::Finished {
                finish_reason,
                usage,
            } => {
                assert_eq!(finish_reason, "stop");
                let usage = usage.as_ref().expect("usage");
                assert_eq!(usage.input_tokens, 3);
                assert_eq!(usage.output_tokens, 2);
            }
            other => panic!("expected Finished, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn function_calls_are_assembled_across_chunks() {
        let events = collect(&[
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"item_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"item_1\",\"delta\":\"{\\\"path\\\":\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"item_1\",\"delta\":\"\\\"a\\\"}\"}\n\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"item_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"a\\\"}\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"a\\\"}\"}]}}\n\n",
        ])
        .await;
        assert!(matches!(
            &events[0],
            StreamEvent::ToolCallStart { id, name } if id == "call_1" && name == "read"
        ));
        assert!(matches!(
            &events[1],
            StreamEvent::ToolCallArgumentDelta { id, .. } if id == "call_1"
        ));
        assert!(matches!(
            &events[2],
            StreamEvent::ToolCallArgumentDelta { id, .. } if id == "call_1"
        ));
        assert!(matches!(
            &events[3],
            StreamEvent::ToolCallFinish { id, name, arguments }
                if id == "call_1" && name == "read" && arguments == "{\"path\":\"a\"}"
        ));
        assert!(matches!(
            &events[4],
            StreamEvent::NativeOutput(items) if items.len() == 1
        ));
        assert!(matches!(
            &events[5],
            StreamEvent::Finished { finish_reason, .. } if finish_reason == "tool_calls"
        ));
    }

    #[tokio::test]
    async fn failed_responses_surface_the_provider_message() {
        let events = collect(&[
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"boom\"}}}\n\n",
        ])
        .await;
        assert!(matches!(&events[0], StreamEvent::Error(message) if message == "boom"));
    }

    #[tokio::test]
    async fn a_done_marker_without_completed_finishes_the_stream() {
        let events = collect(&["data: [DONE]\n\n"]).await;
        assert!(matches!(
            &events[0],
            StreamEvent::Finished { finish_reason, usage }
                if finish_reason == "stop" && usage.is_none()
        ));
    }

    #[tokio::test]
    async fn rate_limit_events_are_consumed_without_ui_noise() {
        let events = collect(&[
            "data: {\"type\":\"codex.rate_limits\",\"rate_limits\":{\"primary\":{\"used_percent\":1,\"window_minutes\":300},\"secondary\":null}}\n\n",
            "data: [DONE]\n\n",
        ])
        .await;
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], StreamEvent::Finished { .. }));
    }
}
