//! OpenAI-Compatible adapter for OpenRouter, domestic gateways, vLLM, etc.
//!
//! Wires the same SSE shape as the [`crate::openai`] adapter but lets the
//! caller pick chat-completions vs responses endpoints at construction time.

pub use crate::openai::{OpenAiProvider, DEFAULT_BASE_URL};

pub type WireApi = super::openai_compat_wire::WireApi;

pub fn openai_compat(
    base_url: impl Into<String>,
    api_key: Option<String>,
    _wire: WireApi,
) -> OpenAiProvider {
    OpenAiProvider::new(base_url, api_key)
}
