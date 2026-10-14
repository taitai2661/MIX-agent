//! Wire API selector for OpenAI-Compatible providers. Phase 1 keeps a single
//! implementation; the Responses endpoint adapter lands in Phase 6.

pub enum WireApi {
    ChatCompletions,
    Responses,
}
