//! Ollama adapter. Ollama exposes an OpenAI-compatible endpoint at
//! `http://host/v1/chat/completions`, so we reuse the OpenAI client with a
//! no-key configuration.

pub use crate::openai::OpenAiProvider;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";

pub fn ollama(base_url: impl Into<String>) -> OpenAiProvider {
    OpenAiProvider::new(base_url, None)
}
