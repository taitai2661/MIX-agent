//! Google Gemini generateContent streaming. Each SSE `data:` payload is a JSON
//! object containing `candidates[0].content.parts[0].text` deltas.

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};

use super::transport;
use super::{
    Provider, ProviderCapabilities, ProviderId, ProviderRequest, ReasoningControl, StreamEvent,
    Usage,
};

pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";

pub struct GeminiProvider {
    pub base_url: String,
    pub api_key: Option<String>,
}

impl GeminiProvider {
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
        }
    }
}

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiContent {
    role: &'static str,
    parts: Vec<GeminiPart>,
}

/// The API spells these keys in camelCase (`functionCall`, `functionResponse`).
/// Without the rename the request is rejected and — worse — the incoming stream
/// silently fails to parse, so the model appears to have stopped calling tools.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<GeminiFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<GeminiFunctionResponse>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiFunctionResponse {
    name: String,
    /// The API requires an object here, so a string result is boxed.
    response: serde_json::Value,
}

#[derive(Serialize)]
struct GeminiTool {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_config: Option<GeminiThinkingConfig>,
}

#[derive(Serialize)]
struct GeminiThinkingConfig {
    thinking_budget: u32,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GeminiChunk {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsage>,
    #[serde(default)]
    #[allow(dead_code)]
    model_version: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContentOwned>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GeminiContentOwned {
    #[serde(default)]
    parts: Vec<GeminiPartOwned>,
    #[serde(default)]
    #[allow(dead_code)]
    role: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GeminiPartOwned {
    #[serde(default)]
    text: Option<String>,
    /// A boolean flag on a part whose text lives in `text`. Deserialising it as
    /// a string made the whole chunk fail to parse — and the parser drops
    /// unparseable chunks silently.
    #[serde(default)]
    thought: Option<bool>,
    #[serde(default)]
    function_call: Option<GeminiFunctionCall>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GeminiUsage {
    #[serde(default)]
    prompt_token_count: u64,
    #[serde(default)]
    candidates_token_count: u64,
    #[serde(default)]
    cached_content_token_count: u64,
    #[serde(default)]
    thoughts_token_count: u64,
}

#[async_trait]
impl Provider for GeminiProvider {
    fn id(&self) -> &ProviderId {
        &self.base_url
    }
    fn label(&self) -> &str {
        "Gemini"
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
            .ok_or_else(|| anyhow::anyhow!("Gemini provider requires an API key"))?;
        let base_url = self.base_url.trim_end_matches('/').to_string();
        let url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            base_url, req.model, api_key
        );

        let (system, contents) = build_contents(&req);
        let tools = if req.tools.is_empty() {
            None
        } else {
            Some(vec![GeminiTool {
                function_declarations: req
                    .tools
                    .iter()
                    .map(|t| GeminiFunctionDeclaration {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: t.parameters.clone(),
                    })
                    .collect(),
            }])
        };
        let generation_config = if req.reasoning.is_some()
            || req.temperature.is_some()
            || req.top_p.is_some()
            || req.max_tokens.is_some()
        {
            let thinking_config = match req.reasoning {
                Some(ReasoningControl::Low) => Some(GeminiThinkingConfig {
                    thinking_budget: 1024,
                }),
                Some(ReasoningControl::Medium) => Some(GeminiThinkingConfig {
                    thinking_budget: 4096,
                }),
                Some(ReasoningControl::High) => Some(GeminiThinkingConfig {
                    thinking_budget: 16384,
                }),
                Some(ReasoningControl::Xhigh) => Some(GeminiThinkingConfig {
                    thinking_budget: 32768,
                }),
                Some(ReasoningControl::Custom { budget_tokens }) => Some(GeminiThinkingConfig {
                    thinking_budget: budget_tokens,
                }),
                _ => None,
            };
            Some(GeminiGenerationConfig {
                temperature: req.temperature,
                top_p: req.top_p,
                max_output_tokens: req.max_tokens,
                thinking_config,
            })
        } else {
            None
        };

        let body = GeminiRequest {
            contents,
            system_instruction: system,
            tools,
            generation_config,
        };

        let response =
            transport::send_with_retry(transport::json_post(transport::client()?, &url, &body))
                .await?;

        let sse = transport::sse_stream(response);
        let buffer = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));

        Ok(Box::pin(ge_sse_stream(buffer, sse)))
    }

    /// `POST {base}/v1beta/models/batchEmbedContents`, Gemini's batch shape.
    ///
    /// One call for the whole batch rather than `embedContent` per text: a
    /// Memory backfill embeds every row, and per-row calls would multiply both
    /// the latency and the quota cost. The model travels as `models/{m}` and a
    /// caller that already passed the prefixed form is not double-prefixed.
    async fn embed(&self, model: &str, texts: &[String]) -> anyhow::Result<Vec<Vec<f64>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let api_key = self
            .api_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Gemini provider requires an API key"))?;
        let base_url = self.base_url.trim_end_matches('/');
        let url = format!(
            "{}/v1beta/models/batchEmbedContents?key={}",
            base_url, api_key
        );
        let model = if model.starts_with("models/") {
            model.to_string()
        } else {
            format!("models/{}", model)
        };

        let body = GeminiBatchEmbedRequest {
            requests: texts
                .iter()
                .map(|text| GeminiEmbedRequest {
                    model: model.clone(),
                    content: GeminiEmbedContent {
                        parts: vec![GeminiEmbedPart { text: text.clone() }],
                    },
                })
                .collect(),
        };
        let response =
            transport::send_with_retry(transport::json_post(transport::client()?, &url, &body))
                .await?;
        let parsed: GeminiBatchEmbedResponse = response
            .json()
            .await
            .map_err(|err| anyhow::anyhow!("batchEmbedContents response JSON: {}", err))?;
        if parsed.embeddings.len() != texts.len() {
            return Err(anyhow::anyhow!(
                "batchEmbedContents returned {} vector(s) for {} input(s)",
                parsed.embeddings.len(),
                texts.len()
            ));
        }
        Ok(parsed.embeddings.into_iter().map(|e| e.values).collect())
    }
}

#[derive(Serialize)]
struct GeminiBatchEmbedRequest {
    requests: Vec<GeminiEmbedRequest>,
}

#[derive(Serialize)]
struct GeminiEmbedRequest {
    model: String,
    content: GeminiEmbedContent,
}

#[derive(Serialize)]
struct GeminiEmbedContent {
    parts: Vec<GeminiEmbedPart>,
}

#[derive(Serialize)]
struct GeminiEmbedPart {
    text: String,
}

#[derive(Deserialize)]
struct GeminiBatchEmbedResponse {
    #[serde(default)]
    embeddings: Vec<GeminiEmbedEntry>,
}

#[derive(Deserialize)]
struct GeminiEmbedEntry {
    #[serde(default)]
    values: Vec<f64>,
}

fn ge_sse_stream(
    buffer: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    sse: std::pin::Pin<Box<dyn Stream<Item = anyhow::Result<Bytes>> + Send>>,
) -> impl Stream<Item = StreamEvent> + Send + 'static {
    let mut sse = sse;
    async_stream::stream! {
        let mut finished_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
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
                if raw.is_empty() { continue; }
                let chunk: GeminiChunk = match serde_json::from_str(&raw) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                if let Some(u) = chunk.usage_metadata {
                    usage = Some(Usage {
                        input_tokens: u.prompt_token_count,
                        // Thinking tokens are neither prompt nor cache tokens;
                        // they belong in the output count.
                        output_tokens: u.candidates_token_count + u.thoughts_token_count,
                        cache_read_tokens: u.cached_content_token_count,
                        cache_write_tokens: 0,
                    });
                }
                for candidate in chunk.candidates {
                    if let Some(content) = candidate.content {
                        for part in content.parts {
                            let is_thought = part.thought.unwrap_or(false);
                            if let Some(text) = part.text {
                                if !text.is_empty() {
                                    if is_thought {
                                        yield StreamEvent::ReasoningDelta(text);
                                    } else {
                                        yield StreamEvent::TextDelta(text);
                                    }
                                }
                            }
                            // Repeated identical calls are legitimate; a
                            // dedup set silently swallowed the second one.
                            if let Some(call) = part.function_call {
                                let id = format!("gemini-{}", uuid::Uuid::new_v4());
                                let args = serde_json::to_string(&call.args).unwrap_or_default();
                                yield StreamEvent::ToolCallStart { id: id.clone(), name: call.name.clone() };
                                yield StreamEvent::ToolCallArgumentDelta { id: id.clone(), delta: args.clone() };
                                yield StreamEvent::ToolCallFinish { id, name: call.name, arguments: args };
                            }
                        }
                    }
                    if let Some(reason) = candidate.finish_reason {
                        if finished_reason.is_none() {
                            finished_reason = Some(reason);
                        }
                    }
                }
                if finished_reason.is_some() && usage.is_some() {
                    yield StreamEvent::Finished {
                        finish_reason: finished_reason.take().unwrap_or_else(|| "stop".into()),
                        usage,
                    };
                    return;
                }
            }
        }
        // The stream can also close before both signals arrive together.
        if let Some(reason) = finished_reason {
            yield StreamEvent::Finished { finish_reason: reason, usage };
        }
    }
}

/// Translate the shared message list into Gemini's `contents` array.
///
/// Gemini has no `system` role inside `contents` (it uses the separate
/// `system_instruction` field) and no `tool` role: a tool result is a `user`
/// turn whose part is a `functionResponse`. Getting this wrong is not a
/// cosmetic bug — the model never sees its own function call again and cannot
/// connect a result to the question that produced it.
fn build_contents(req: &ProviderRequest) -> (Option<GeminiContent>, Vec<GeminiContent>) {
    let mut contents: Vec<GeminiContent> = Vec::new();
    let mut system_parts: Vec<String> = Vec::new();
    if let Some(system) = req.system.as_deref() {
        if !system.trim().is_empty() {
            system_parts.push(system.to_string());
        }
    }

    // Function names called by the most recent model turn. A `tool` message is
    // only kept when its name appears here: Gemini rejects a functionResponse
    // that does not match a preceding functionCall, which is exactly the shape a
    // resumed checkpoint produces.
    let mut awaited: Vec<String> = Vec::new();

    for m in &req.messages {
        match m.role.as_str() {
            "system" => {
                if !m.content.trim().is_empty() {
                    system_parts.push(m.content.clone());
                }
            }
            "tool" => {
                let Some(name) = m.name.as_deref() else {
                    continue;
                };
                if !awaited.iter().any(|n| n == name) {
                    continue;
                }
                let response = match serde_json::from_str::<serde_json::Value>(&m.content) {
                    Ok(parsed) if parsed.is_object() => parsed,
                    _ => serde_json::json!({
                        "result": m.content,
                        "is_error": m.is_error,
                    }),
                };
                let part = GeminiPart {
                    text: None,
                    function_call: None,
                    function_response: Some(GeminiFunctionResponse {
                        name: name.to_string(),
                        response,
                    }),
                };
                // Results for parallel calls belong in one user turn.
                match contents.last_mut() {
                    Some(last) if last.role == "user" && is_function_response_turn(last) => {
                        last.parts.push(part);
                    }
                    _ => contents.push(GeminiContent {
                        role: "user",
                        parts: vec![part],
                    }),
                }
            }
            "assistant" => {
                let mut parts = Vec::new();
                if !m.content.is_empty() {
                    parts.push(GeminiPart {
                        text: Some(m.content.clone()),
                        function_call: None,
                        function_response: None,
                    });
                }
                for call in &m.tool_calls {
                    let args = serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                        .unwrap_or_else(|_| serde_json::json!({}));
                    parts.push(GeminiPart {
                        text: None,
                        function_call: Some(GeminiFunctionCall {
                            name: call.function.name.clone(),
                            args,
                        }),
                        function_response: None,
                    });
                }
                if parts.is_empty() {
                    continue;
                }
                awaited = m
                    .tool_calls
                    .iter()
                    .map(|c| c.function.name.clone())
                    .collect();
                contents.push(GeminiContent {
                    role: "model",
                    parts,
                });
            }
            _ => {
                awaited.clear();
                if m.content.is_empty() {
                    continue;
                }
                contents.push(GeminiContent {
                    role: "user",
                    parts: vec![GeminiPart {
                        text: Some(m.content.clone()),
                        function_call: None,
                        function_response: None,
                    }],
                });
            }
        }
    }

    // The API rejects a history that does not start with a user turn, which is
    // what a resumed checkpoint can produce.
    while matches!(contents.first(), Some(first) if first.role == "model") {
        contents.remove(0);
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(GeminiContent {
            role: "system",
            parts: vec![GeminiPart {
                text: Some(system_parts.join("\n\n")),
                function_call: None,
                function_response: None,
            }],
        })
    };
    (system, contents)
}

fn is_function_response_turn(content: &GeminiContent) -> bool {
    !content.parts.is_empty() && content.parts.iter().all(|p| p.function_response.is_some())
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
        ge_sse_stream(buffer, feed(chunks)).collect().await
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
    async fn a_camel_case_final_chunk_finishes_and_reports_usage() {
        let events = collect_sse(&[
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":5,\"thoughtsTokenCount\":3}}\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"!\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":9,\"thoughtsTokenCount\":3}}\n",
        ])
        .await;
        let done = finished(&events);
        assert_eq!(done.len(), 1, "camelCase fields must parse: {events:?}");
        assert_eq!(done[0].0, "STOP");
        let usage = done[0].1.clone().expect("usage must be reported");
        assert_eq!(usage.input_tokens, 7);
        assert_eq!(usage.output_tokens, 12);
    }

    #[tokio::test]
    async fn a_boolean_thought_part_is_routed_to_reasoning() {
        let events = collect_sse(&[ "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"pondering\",\"thought\":true},{\"text\":\"answer\"}]}}]}\n" ]).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ReasoningDelta(t) if t == "pondering")),
            "thought parts are reasoning: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::TextDelta(t) if t == "answer")),
            "non-thought parts are text: {events:?}"
        );
    }

    #[tokio::test]
    async fn repeated_identical_tool_calls_are_both_emitted() {
        let events = collect_sse(&[ "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"files\",\"args\":{\"path\":\"a\"}}},{\"functionCall\":{\"name\":\"files\",\"args\":{\"path\":\"a\"}}}]}}]}\n" ]).await;
        let calls = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::ToolCallFinish { .. }))
            .count();
        assert_eq!(calls, 2, "a deliberate repeat must not be swallowed");
    }

    fn request(messages: Vec<Message>) -> ProviderRequest {
        ProviderRequest {
            model: "gemini".into(),
            messages,
            system: None,
            tools: std::sync::Arc::new(Vec::new()),
            temperature: None,
            top_p: None,
            max_tokens: None,
            reasoning: None,
            stream: true,
            context_window: None,
            keep_alive: None,
        }
    }

    fn tool_call(name: &str) -> AssistantToolCall {
        AssistantToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: AssistantToolCallFn {
                name: name.into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            },
        }
    }

    #[test]
    fn assistant_function_call_travels_back_to_the_api() {
        let (_, contents) = build_contents(&request(vec![
            Message::text("user", "edit a.rs"),
            Message::assistant_with_tools("", vec![tool_call("edit")]),
        ]));
        let model = contents.last().expect("model turn");
        assert_eq!(model.role, "model");
        let call = model.parts[0]
            .function_call
            .as_ref()
            .expect("functionCall part");
        assert_eq!(call.name, "edit");
        assert_eq!(call.args["path"], "a.rs");
    }

    #[test]
    fn tool_result_becomes_a_function_response() {
        let call = tool_call("edit");
        let (_, contents) = build_contents(&request(vec![
            Message::text("user", "edit a.rs"),
            Message::assistant_with_tools("", vec![call.clone()]),
            Message::tool_result(&call, "plain text result", false),
        ]));
        assert_eq!(contents.len(), 3);
        let last = contents.last().expect("response turn");
        assert_eq!(last.role, "user");
        let response = last.parts[0]
            .function_response
            .as_ref()
            .expect("functionResponse part");
        assert_eq!(response.name, "edit");
        // The API requires an object, so a bare string is boxed.
        assert_eq!(response.response["result"], "plain text result");
    }

    #[test]
    fn an_object_result_is_passed_through_unboxed() {
        let call = tool_call("edit");
        let (_, contents) = build_contents(&request(vec![
            Message::text("user", "go"),
            Message::assistant_with_tools("", vec![call.clone()]),
            Message::tool_result(&call, r#"{"path":"a.rs","replacements":1}"#, false),
        ]));
        let response = contents[2].parts[0]
            .function_response
            .as_ref()
            .expect("functionResponse part");
        assert_eq!(response.response["replacements"], 1);
        assert!(response.response.get("result").is_none());
    }

    #[test]
    fn a_failed_result_records_the_error_flag() {
        let call = tool_call("edit");
        let (_, contents) = build_contents(&request(vec![
            Message::text("user", "go"),
            Message::assistant_with_tools("", vec![call.clone()]),
            Message::tool_result(&call, "old_string was not found", true),
        ]));
        let response = contents[2].parts[0]
            .function_response
            .as_ref()
            .expect("functionResponse part");
        assert_eq!(response.response["is_error"], true);
    }

    #[test]
    fn parallel_results_merge_into_one_user_turn() {
        let a = tool_call("grep");
        let b = tool_call("glob");
        let (_, contents) = build_contents(&request(vec![
            Message::text("user", "go"),
            Message::assistant_with_tools("", vec![a.clone(), b.clone()]),
            Message::tool_result(&a, "one", false),
            Message::tool_result(&b, "two", false),
        ]));
        assert_eq!(contents.len(), 3, "two results must share one user turn");
        assert_eq!(contents[2].parts.len(), 2);
    }

    #[test]
    fn system_messages_go_to_system_instruction() {
        let (system, contents) = build_contents(&request(vec![
            Message::text("system", "<task_state>goal</task_state>"),
            Message::text("user", "hi"),
        ]));
        let instruction = system.expect("system instruction");
        assert!(instruction.parts[0]
            .text
            .as_deref()
            .expect("text")
            .contains("<task_state>"));
        assert_eq!(contents.len(), 1);
    }

    #[test]
    fn a_leading_model_turn_is_dropped() {
        let (_, contents) = build_contents(&request(vec![
            Message::text("assistant", "orphan"),
            Message::text("user", "hi"),
        ]));
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].role, "user");
    }

    #[test]
    fn a_trailing_orphan_response_is_dropped() {
        let (_, contents) = build_contents(&request(vec![
            Message::text("user", "hi"),
            Message {
                role: "tool".into(),
                content: "orphan".into(),
                tool_calls: Vec::new(),
                tool_call_id: Some("x".into()),
                name: Some("edit".into()),
                is_error: false,
            },
        ]));
        assert_eq!(contents.len(), 1);
    }

    #[test]
    fn a_tool_result_without_a_name_is_skipped() {
        // Gemini matches a response to its call by name; an unnamed one would be
        // rejected as an unmatched response.
        let (_, contents) = build_contents(&request(vec![
            Message::text("user", "go"),
            Message::assistant_with_tools("", vec![tool_call("edit")]),
            Message {
                role: "tool".into(),
                content: "result".into(),
                tool_calls: Vec::new(),
                tool_call_id: Some("call_1".into()),
                name: None,
                is_error: false,
            },
        ]));
        assert_eq!(contents.len(), 2);
    }

    #[test]
    fn serialised_function_call_has_the_wire_shape() {
        let (_, contents) = build_contents(&request(vec![
            Message::text("user", "go"),
            Message::assistant_with_tools("", vec![tool_call("edit")]),
        ]));
        let json = serde_json::to_value(&contents[1]).expect("serialize");
        assert_eq!(json["role"], "model");
        assert_eq!(json["parts"][0]["functionCall"]["name"], "edit");
    }
}
