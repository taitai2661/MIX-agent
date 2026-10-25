//! OpenAI Chat Completions streaming. Phase 1 uses `chat/completions` (not the
//! newer Responses endpoint) so the same code path serves OpenAI-compat
//! gateways, Ollama, and LM Studio.

use async_trait::async_trait;
use futures_util::stream::StreamExt;
use serde::{Deserialize, Serialize};

use super::transport;
use super::{
    AssistantToolCall, Provider, ProviderCapabilities, ProviderId, ProviderRequest,
    ReasoningControl, StreamEvent, Usage,
};

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

pub struct OpenAiProvider {
    pub base_url: String,
    pub api_key: Option<String>,
}

impl OpenAiProvider {
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
        }
    }
}

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    messages: &'a [OpenAiMessage<'a>],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [OpenAiTool]>,
    stream: bool,
    /// `include_usage` makes the API append a final chunk that carries the
    /// token counts; without it every count stays at zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<OpenAiStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
}

#[derive(Serialize)]
struct OpenAiStreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<&'a [AssistantToolCall]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    kind: &'static str,
    function: OpenAiToolFn,
}

#[derive(Serialize)]
struct OpenAiToolFn {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize, Default)]
struct OpenAiChunk {
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize, Default)]
struct OpenAiChoice {
    #[serde(default)]
    delta: OpenAiDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct OpenAiDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAiDeltaToolCall>,
}

#[derive(Deserialize, Default)]
struct OpenAiDeltaToolCall {
    #[serde(default)]
    index: i64,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: OpenAiDeltaToolCallFn,
}

#[derive(Deserialize, Default)]
struct OpenAiDeltaToolCallFn {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize, Default)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: OpenAiUsageDetails,
}

#[derive(Deserialize, Default)]
#[allow(dead_code)]
struct OpenAiUsageDetails {
    #[serde(default)]
    cached_tokens: u64,
    #[serde(default)]
    reasoning_tokens: u64,
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn id(&self) -> &ProviderId {
        &self.base_url
    }
    fn label(&self) -> &str {
        "OpenAI"
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
        let base_url = self.base_url.trim_end_matches('/').to_string();
        let url = format!("{}/chat/completions", base_url);
        // Local runtimes (Ollama, LM Studio) legitimately have no key, so only
        // send Authorization when one was actually configured.
        let requires_key = !is_local_endpoint(&base_url);
        if requires_key && self.api_key.is_none() {
            return Err(anyhow::anyhow!(
                "OpenAI provider requires an API key (or a local endpoint such as http://localhost:11434)"
            ));
        }

        let messages = build_messages(&req);
        let tools = if req.tools.is_empty() {
            None
        } else {
            Some(
                req.tools
                    .iter()
                    .map(|t| OpenAiTool {
                        kind: "function",
                        function: OpenAiToolFn {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            parameters: t.parameters.clone(),
                        },
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let effort = match req.reasoning {
            Some(ReasoningControl::Low) => Some("low"),
            Some(ReasoningControl::Medium) => Some("medium"),
            Some(ReasoningControl::High) => Some("high"),
            Some(ReasoningControl::Xhigh) => Some("xhigh"),
            _ => None,
        };
        let body = OpenAiRequest {
            model: &req.model,
            messages: &messages,
            tools: tools.as_deref(),
            stream: true,
            stream_options: Some(OpenAiStreamOptions {
                include_usage: true,
            }),
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_tokens,
            reasoning_effort: effort,
        };

        let mut request = transport::json_post(transport::client(), &url, &body);
        if let Some(api_key) = self.api_key.as_deref().filter(|k| !k.is_empty()) {
            request = request.bearer_auth(api_key);
        }
        // Retries transient failures; a permanent 400 surfaces immediately.
        let response = transport::send_with_retry(request).await?;

        let buffer = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
        let sse = transport::sse_stream(response);

        Ok(Box::pin(async_stream::stream! {
            let sse = sse;
            #[allow(unused_mut)]
            let mut tool_state: std::collections::HashMap<i64, (String, String, String)> = Default::default();
            #[allow(unused_mut)]
            let mut sse = sse;
            // `finish_reason` and `usage` arrive in different chunks: the finish
            // chunk has no usage, and the usage chunk has an empty `choices`
            // array, so both had to be remembered instead of emitting `Finished`
            // on the first one.
            let mut finish_reason: Option<String> = None;
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
                    if transport::sse_done(&raw) {
                        yield StreamEvent::Finished {
                            finish_reason: finish_reason.take().unwrap_or_else(|| "stop".into()),
                            usage,
                        };
                        return;
                    }
                    let chunk: OpenAiChunk = match serde_json::from_str(&raw) {
                        Ok(c) => c,
                        Err(e) => {
                            yield StreamEvent::Error(format!("SSE JSON: {}", e));
                            continue;
                        }
                    };
                    if let Some(u) = chunk.usage {
                        usage = Some(Usage {
                            input_tokens: u.prompt_tokens,
                            output_tokens: u.completion_tokens,
                            cache_read_tokens: u.prompt_tokens_details.cached_tokens,
                            // The API reports no cache-write count.
                            cache_write_tokens: 0,
                        });
                    }
                    for choice in chunk.choices {
                        if let Some(text) = choice.delta.content {
                            if !text.is_empty() {
                                yield StreamEvent::TextDelta(text);
                            }
                        }
                        if let Some(reasoning) = choice.delta.reasoning_content {
                            if !reasoning.is_empty() {
                                yield StreamEvent::ReasoningDelta(reasoning);
                            }
                        }
                        for tc in choice.delta.tool_calls {
                            let entry = tool_state.entry(tc.index).or_insert_with(|| {
                                (String::new(), String::new(), String::new())
                            });
                            if let Some(id) = tc.id {
                                if entry.0.is_empty() {
                                    entry.0 = id.clone();
                                    if let Some(name) = tc.function.name.clone() {
                                        entry.1 = name.clone();
                                        yield StreamEvent::ToolCallStart { id, name };
                                    }
                                }
                            }
                            if let Some(name) = tc.function.name {
                                if !entry.1.contains(&name) && entry.1.is_empty() {
                                    entry.1 = name.clone();
                                    if !entry.0.is_empty() {
                                        yield StreamEvent::ToolCallStart {
                                            id: entry.0.clone(),
                                            name,
                                        };
                                    }
                                }
                            }
                            if let Some(args) = tc.function.arguments {
                                if !args.is_empty() {
                                    entry.2.push_str(&args);
                                    yield StreamEvent::ToolCallArgumentDelta {
                                        id: entry.0.clone(),
                                        delta: args,
                                    };
                                }
                            }
                        }
                        if let Some(reason) = choice.finish_reason {
                            if finish_reason.is_none() {
                                finish_reason = Some(reason);
                            }
                            for (_, (id, name, args)) in tool_state.drain() {
                                if !id.is_empty() {
                                    yield StreamEvent::ToolCallFinish { id, name, arguments: args };
                                }
                            }
                        }
                    }
                    if finish_reason.is_some() && usage.is_some() {
                        yield StreamEvent::Finished {
                            finish_reason: finish_reason.take().unwrap_or_else(|| "stop".into()),
                            usage,
                        };
                        return;
                    }
                }
            }
            // The connection closed without `[DONE]`.
            if let Some(reason) = finish_reason {
                yield StreamEvent::Finished { finish_reason: reason, usage };
            }
        }))
    }

    /// `POST {base}/embeddings`, the OpenAI shape.
    ///
    /// Ollama, LM Studio and every OpenAI-compatible gateway reuse this
    /// adapter, so one implementation covers all four transports: they all
    /// answer `{"data":[{"index":0,"embedding":[...]}]}` for
    /// `{"model":..., "input":[...]}`. `index` is honoured rather than assumed
    /// to be positional — a gateway is free to return the batch out of order,
    /// and a silently shuffled batch would attach each vector to the wrong row.
    async fn embed(&self, model: &str, texts: &[String]) -> anyhow::Result<Vec<Vec<f64>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let base_url = self.base_url.trim_end_matches('/');
        let url = format!("{}/embeddings", base_url);
        // Same key rule as `stream`: a loopback runtime legitimately runs with
        // no key, a cloud endpoint does not.
        if !is_local_endpoint(base_url) && self.api_key.is_none() {
            return Err(anyhow::anyhow!(
                "OpenAI provider requires an API key (or a local endpoint such as http://localhost:11434)"
            ));
        }

        let body = EmbeddingsRequest {
            model,
            input: texts,
        };
        let mut request = transport::json_post(transport::client(), &url, &body);
        if let Some(api_key) = self.api_key.as_deref().filter(|k| !k.is_empty()) {
            request = request.bearer_auth(api_key);
        }
        let response = transport::send_with_retry(request).await?;
        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|err| anyhow::anyhow!("embeddings response JSON: {}", err))?;
        parse_embeddings(payload, texts.len())
    }
}

/// Turn an `/embeddings` payload into vectors, in input order.
///
/// Split out so the ordering and the cardinality check are testable without a
/// live endpoint: a batch that comes back shuffled, or short, must fail loudly
/// — silently accepting it would store each vector against the wrong Memory.
fn parse_embeddings(payload: serde_json::Value, expected: usize) -> anyhow::Result<Vec<Vec<f64>>> {
    let parsed: EmbeddingsResponse = serde_json::from_value(payload)
        .map_err(|err| anyhow::anyhow!("embeddings response JSON: {}", err))?;
    if parsed.data.len() != expected {
        return Err(anyhow::anyhow!(
            "embeddings endpoint returned {} vector(s) for {} input(s)",
            parsed.data.len(),
            expected
        ));
    }
    let mut entries = parsed.data;
    entries.sort_by_key(|entry| entry.index);
    Ok(entries.into_iter().map(|entry| entry.embedding).collect())
}

#[derive(Serialize)]
struct EmbeddingsRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbeddingsResponse {
    #[serde(default)]
    data: Vec<EmbeddingsEntry>,
}

#[derive(Deserialize)]
struct EmbeddingsEntry {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    embedding: Vec<f64>,
}

/// True for loopback / private-network endpoints, where an API key is usually
/// not configured (Ollama, LM Studio, vLLM, a LAN inference box).
///
/// Single source of truth lives in [`crate::discovery`]: `host_of` parses the
/// URL and `is_loopback_host` decides, so the key gate and the scanner can
/// never disagree about what "local" means (an IPv6-loopback Ollama once
/// demanded a key because a copy of this logic split `[::1]` on `':'`).
fn is_local_endpoint(base_url: &str) -> bool {
    crate::discovery::is_loopback_host(crate::discovery::host_of(base_url))
}

fn build_messages(req: &ProviderRequest) -> Vec<OpenAiMessage<'_>> {
    let mut out: Vec<OpenAiMessage<'_>> = Vec::new();
    if let Some(system) = &req.system {
        if !system.is_empty() {
            out.push(OpenAiMessage {
                role: "system",
                content: Some(system.as_str()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }
    }
    for idx in super::resolved_history_indices(&req.messages) {
        let m = &req.messages[idx];
        out.push(OpenAiMessage {
            role: m.role.as_str(),
            content: if m.content.is_empty() {
                None
            } else {
                Some(m.content.as_str())
            },
            tool_calls: if m.tool_calls.is_empty() {
                None
            } else {
                Some(&m.tool_calls)
            },
            tool_call_id: m.tool_call_id.as_deref(),
            name: m.name.as_deref(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{build_messages, is_local_endpoint, parse_embeddings};
    use crate::{AssistantToolCall, AssistantToolCallFn, Message, ProviderRequest};

    #[test]
    fn detects_local_endpoints() {
        for url in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1",
            "http://0.0.0.0:8000/v1",
            "http://10.0.0.5:8080/v1",
            "http://192.168.1.20:11434/v1",
            "http://172.16.4.4:8000/v1",
            "http://172.31.255.1/v1",
        ] {
            assert!(is_local_endpoint(url), "{url} should be local");
        }
    }

    #[test]
    fn cloud_endpoints_are_not_local() {
        for url in [
            "https://api.openai.com/v1",
            "https://api.groq.com/openai/v1",
            "https://openrouter.ai/api/v1",
            "https://172.15.0.1/v1",
            "https://172.32.0.1/v1",
            "https://11.0.0.1/v1",
            "https://192.169.1.1/v1",
        ] {
            assert!(!is_local_endpoint(url), "{url} should not be local");
        }
    }

    fn request(messages: Vec<Message>) -> ProviderRequest {
        ProviderRequest {
            model: "gpt".into(),
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

    #[test]
    fn assistant_tool_calls_survive_a_round_trip() {
        let call = AssistantToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: AssistantToolCallFn {
                name: "edit".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            },
        };
        let req = request(vec![
            Message::text("user", "edit a.rs"),
            Message::assistant_with_tools("", vec![call.clone()]),
            Message::tool_result(&call, "ok", false),
        ]);
        let messages = build_messages(&req);
        let assistant = messages
            .iter()
            .find(|m| m.role == "assistant")
            .expect("assistant message");
        let calls = assistant.tool_calls.expect("tool_calls must be preserved");
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "edit");
        let tool = messages.last().expect("tool message");
        assert_eq!(tool.tool_call_id, Some("call_1"));
    }

    #[test]
    fn an_orphan_tool_message_is_dropped() {
        // This is the exact shape `load_history` used to rehydrate: a `tool`
        // message whose assistant turn had lost its tool_calls. OpenAI answers
        // 400 for it, which broke every conversation containing a tool call.
        let req = request(vec![
            Message::text("user", "go"),
            Message {
                role: "tool".into(),
                content: "ok".into(),
                tool_calls: Vec::new(),
                tool_call_id: Some("call_1".into()),
                name: Some("edit".into()),
                is_error: false,
            },
        ]);
        let messages = build_messages(&req);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
    }

    #[test]
    fn a_system_prompt_is_prepended() {
        let mut req = request(vec![Message::text("user", "hi")]);
        req.system = Some("be brief".into());
        let messages = build_messages(&req);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content, Some("be brief"));
    }

    /// A gateway may answer the batch in any order: the vectors have to be
    /// matched back to their inputs by `index`, never by position.
    #[test]
    fn an_out_of_order_embeddings_batch_is_reordered() {
        let payload = serde_json::json!({
            "data": [
                { "index": 1, "embedding": [0.0, 1.0] },
                { "index": 0, "embedding": [1.0, 0.0] }
            ]
        });
        let out = parse_embeddings(payload, 2).expect("parse");
        assert_eq!(out, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    }

    /// A short batch would attach vectors to the wrong rows, so it is an error
    /// rather than something to zip up optimistically.
    #[test]
    fn a_short_embeddings_batch_is_rejected() {
        let payload = serde_json::json!({ "data": [ { "index": 0, "embedding": [1.0] } ] });
        let err = parse_embeddings(payload, 2).expect_err("one vector for two inputs");
        assert!(err.to_string().contains("1 vector(s) for 2 input(s)"));
    }

    /// Every transport except Anthropic reuses this adapter, so its default
    /// embedding model is the one thing that differs between them.
    #[test]
    fn each_transport_names_its_own_default_embedding_model() {
        use crate::preset::{default_embedding_model, ProviderKind};
        assert_eq!(
            ProviderKind::OpenAI.default_embedding_model(),
            Some("text-embedding-3-small")
        );
        assert_eq!(
            ProviderKind::Ollama.default_embedding_model(),
            Some("nomic-embed-text")
        );
        assert_eq!(
            ProviderKind::LMStudio.default_embedding_model(),
            Some("text-embedding-nomic-embed-text-v1.5")
        );
        assert_eq!(
            ProviderKind::Gemini.default_embedding_model(),
            Some("text-embedding-004")
        );
        assert_eq!(ProviderKind::Anthropic.default_embedding_model(), None);
        assert_eq!(default_embedding_model("anthropic"), None);
        assert_eq!(default_embedding_model("not-a-kind"), None);
    }
}
