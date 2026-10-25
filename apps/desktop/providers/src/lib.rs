//! LLM Provider adapters.
//!
//! Each adapter implements the [`Provider`] trait, exposing `stream` (a stream
//! of [`StreamEvent`] values) and `embed` (batch text embeddings for the memory
//! spec's semantic ranking). The Run Engine in [`mixd-core`] normalises the
//! stream events into the shared SSE-equivalent contract.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};

pub mod anthropic;
pub mod discovery;
pub mod dsml;
pub mod gemini;
pub mod lmstudio;
pub mod ollama;
pub mod openai;
pub mod openai_compat;
pub mod openai_compat_wire;
pub mod preset;
pub mod reliability;
pub mod transport;

pub type ProviderId = String;
pub type ModelId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRequest {
    pub model: ModelId,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub system: Option<String>,
    /// Shared so a run's turns all point at one spec list; cloning the request
    /// per turn no longer deep-copies every tool schema.
    #[serde(default)]
    pub tools: Arc<Vec<ToolSpec>>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub reasoning: Option<ReasoningControl>,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<AssistantToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Set on `tool` messages whose call failed. Adapters translate this into
    /// their own error channel (Anthropic `is_error`, Gemini `error` response),
    /// and the stagnation detector uses it to notice a stuck agent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

impl Message {
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
            is_error: false,
        }
    }

    /// An assistant turn that requested one or more tools.
    pub fn assistant_with_tools(
        text: impl Into<String>,
        tool_calls: Vec<AssistantToolCall>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content: text.into(),
            tool_calls,
            tool_call_id: None,
            name: None,
            is_error: false,
        }
    }

    /// The result of executing one tool call.
    pub fn tool_result(
        call: &AssistantToolCall,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call.id.clone()),
            name: Some(call.function.name.clone()),
            is_error,
        }
    }
}

/// Indices of the messages that survive the tool-result match.
///
/// Every major provider rejects an unanswered `tool` message: OpenAI says a
/// `tool` message "must be a response to a preceding message with
/// `tool_calls`", Anthropic rejects a `tool_result` with no `tool_use`, and
/// Gemini rejects an unmatched `functionResponse`. Rehydrating a conversation
/// from the database, or resuming from a checkpoint, routinely produces exactly
/// that shape, so the check runs once for all adapters.
///
/// The match is by id *or* by function name, because Gemini keys on the name
/// while the other two key on the id. Adapters with stricter rules re-check with
/// their own key.
///
/// Indices rather than a filtered `Vec` so callers can keep borrowing from the
/// request they were handed.
pub fn resolved_history_indices(messages: &[Message]) -> Vec<usize> {
    let mut keep: Vec<usize> = Vec::with_capacity(messages.len());
    // Ids and names the most recent assistant turn is waiting on.
    let mut awaited_ids: Vec<String> = Vec::new();
    let mut awaited_names: Vec<String> = Vec::new();
    for (idx, m) in messages.iter().enumerate() {
        match m.role.as_str() {
            "assistant" => {
                awaited_ids = m.tool_calls.iter().map(|c| c.id.clone()).collect();
                awaited_names = m
                    .tool_calls
                    .iter()
                    .map(|c| c.function.name.clone())
                    .collect();
                keep.push(idx);
            }
            "tool" => {
                let id_ok = m
                    .tool_call_id
                    .as_deref()
                    .is_some_and(|id| awaited_ids.iter().any(|a| a == id));
                let name_ok = m
                    .name
                    .as_deref()
                    .is_some_and(|n| awaited_names.iter().any(|a| a == n));
                if id_ok || name_ok {
                    keep.push(idx);
                }
            }
            // System messages are neither awaited nor awaiting; they pass
            // through without closing the window.
            "system" => keep.push(idx),
            _ => {
                // Any other turn closes the window: a result that arrives after
                // it can no longer be matched to a call.
                awaited_ids.clear();
                awaited_names.clear();
                keep.push(idx);
            }
        }
    }
    keep
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: AssistantToolCallFn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantToolCallFn {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningControl {
    #[default]
    Off,
    Auto,
    Low,
    Medium,
    High,
    Xhigh,
    Custom {
        budget_tokens: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallArgumentDelta {
        id: String,
        delta: String,
    },
    ToolCallFinish {
        id: String,
        name: String,
        arguments: String,
    },
    Finished {
        finish_reason: String,
        usage: Option<Usage>,
    },
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub vision: bool,
    pub tool_calling: bool,
    pub reasoning: bool,
    pub streaming: bool,
}

/// Returned by [`Provider::embed`] when a transport has no embeddings endpoint.
///
/// Anthropic is the standing example: it offers no embedding model, so its
/// adapter keeps the trait's default implementation and this error is what the
/// caller sees. The type is deliberately a distinct value rather than a plain
/// `anyhow!` string so callers can *classify* the failure instead of matching
/// on prose — memory retrieval has to fall back to lexical ranking rather than
/// treat a missing endpoint as a broken provider.
///
/// [`Provider::embed`]: Provider::embed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingNotSupported;

impl std::fmt::Display for EmbeddingNotSupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this provider does not support embeddings")
    }
}

impl std::error::Error for EmbeddingNotSupported {}

/// Whether `err` was caused by a transport that has no embeddings endpoint.
///
/// Walks the whole `anyhow` chain, so a caller that added `.context(..)` still
/// recognises the condition.
pub fn is_embedding_not_supported(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<EmbeddingNotSupported>())
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn label(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    async fn stream(&self, req: ProviderRequest) -> anyhow::Result<StreamBox>;

    /// Embed `texts` with `model`, returning one vector per input, in order.
    ///
    /// Phase 1 of the memory spec (`docs/memory-spec.md` §5) scores a Memory
    /// row with a `vector` term only when both sides can be embedded. The
    /// default implementation answers [`EmbeddingNotSupported`], so adding this
    /// method to the trait did not force a change on every adapter — and, more
    /// importantly, **every failure here must be recoverable by the caller**:
    /// retrieval falls back to lexical-only ranking whenever this returns an
    /// `Err`, so a provider that cannot embed still returns memories, just
    /// without the semantic term.
    async fn embed(&self, model: &str, texts: &[String]) -> anyhow::Result<Vec<Vec<f64>>> {
        let _ = (model, texts);
        Err(EmbeddingNotSupported.into())
    }
}

pub type StreamBox = std::pin::Pin<Box<dyn Stream<Item = StreamEvent> + Send>>;
