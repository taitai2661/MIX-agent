"""Conservative model-role metadata used by local Auto routing.

Provider model listings can mix chat models with endpoint-specific models. Only
known special-purpose IDs are excluded; unknown models remain Auto-eligible.
"""


NON_CHAT_MODEL_PREFIXES = {
    # Model IDs in this namespace are specific to NVIDIA NIM even though its
    # provider wire kind is the generic ``compatible`` transport.
    "*": (
        "nvidia/nemotron-parse",
        "nvidia/nv-embed",
        "nvidia/nv-rerank",
        "nvidia/parakeet",
        # Embedding / retrieval / reranking families (never chat-capable).
        "nvidia/llama-nemotron-embed",
        "nvidia/llama-nemotron-rerank",
        "nvidia/rerank-",
        "nvidia/llama-3.2-nemoretriever",
        "nvidia/gliner",
        "baai/bge-",
        # Image / video / audio generation and perception models.
        "black-forest-labs/flux",
        "nvidia/cosmos-transfer",
        "nvidia/cosmos-predict",
        "nvidia/sparsedrive",
        "nvidia/bevformer",
        "nvidia/streampetr",
        "nvidia/active-speaker-detection",
        "nvidia/synthetic-video-detector",
        "nvidia/magpie-tts",
        "nvidia/studiovoice",
        # Task-specific classifiers and scene-code models that answer with
        # labels rather than conversational text.
        "nvidia/usdcode",
        "nvidia/usdvalidate",
        "nvidia/nemotron-content-safety",
        "nvidia/nemotron-3-content-safety",
        "nvidia/llama-3.1-nemotron-safety-guard",
        "meta/esmfold",
        "meta/esm2-",
        # Translation / recurrent models that reject chat completions.
        "nvidia/riva-translate-4b-instruct",
        "google/recurrentgemma-2b",
    ),
}


def chat_capability(kind, model_id):
    """Return a known chat capability, or ``None`` when it is unknown."""
    identifier = (model_id or "").casefold()
    prefixes = (*NON_CHAT_MODEL_PREFIXES.get("*", ()), *NON_CHAT_MODEL_PREFIXES.get(kind, ()))
    if any(identifier.startswith(prefix) for prefix in prefixes):
        return False
    return None


def is_auto_chat_eligible(model_data, provider_kind=None):
    """Whether a model can participate in Auto's normal conversation routing."""
    capabilities = model_data.get("capabilities", {})
    if capabilities.get("chat") is False or model_data.get("overrides", {}).get("chat") is False:
        return False
    return chat_capability(provider_kind, model_data.get("model_id")) is not False
