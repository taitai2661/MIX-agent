//! LM Studio adapter. LM Studio exposes an OpenAI-compatible endpoint and
//! does not require an API key.

pub use crate::openai::OpenAiProvider;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:1234/v1";

pub fn lmstudio(base_url: impl Into<String>) -> OpenAiProvider {
    OpenAiProvider::new(base_url, None)
}
