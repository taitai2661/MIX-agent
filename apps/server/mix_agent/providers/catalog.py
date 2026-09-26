"""The public provider catalog; wire protocols remain a small, audited set."""

from copy import deepcopy

KIND_VALUES = ("openai", "anthropic", "gemini", "openrouter", "ollama", "lmstudio", "compatible")
CUSTOM_KINDS = ("compatible", "anthropic", "gemini")
TRANSPORT_IDS = ("openai_responses", "openai_compatible", "anthropic_messages",
                 "gemini_generate_content", "ollama", "lmstudio")


def preset(id, name, category, kind, url="", api_key=True, private=False, *, transport_id=None,
           discovery_id=None, extra_config_schema=(), model_transports=None, session_header=None):
    transport_id = transport_id or {
        "openai": "openai_responses", "anthropic": "anthropic_messages",
        "gemini": "gemini_generate_content", "ollama": "ollama", "lmstudio": "lmstudio",
        "openrouter": "openai_compatible", "compatible": "openai_compatible",
    }[kind]
    discovery_id = discovery_id or {
        "openai": "openai_models", "anthropic": "anthropic_models",
        "gemini": "gemini_models", "ollama": "ollama_tags", "lmstudio": "openai_models",
        "openrouter": "openai_models", "compatible": "openai_models",
    }[kind]
    return {"id": id, "name": name, "category": category, "kind": kind,
            "default_url": url, "api_key_required": api_key, "allow_private_default": private,
            "transport_id": transport_id, "discovery_id": discovery_id,
            "metadata_resolver_ids": ("provider_api", "model_info"),
            "extra_config_schema": list(extra_config_schema),
            "model_transports": dict(model_transports or {}),
            "session_header": session_header}


def _endpoint_transports(responses=(), messages=()):
    """Map model ids served on a single non-default gateway endpoint."""
    return {**{model: "openai_responses" for model in responses},
            **{model: "anthropic_messages" for model in messages}}


# OpenCode publishes exactly one endpoint per model and serves the model there
# only: /chat/completions for the OpenAI-compatible families, /responses for
# OpenAI-style models and /messages for Anthropic-style ones.  Every other model
# keeps the preset default.  Sources: https://opencode.ai/docs/go and
# https://opencode.ai/docs/zen ("Endpoints").
OPENCODE_GO_MODEL_TRANSPORTS = _endpoint_transports(
    responses=("grok-4.7", "grok-4.6", "gpt-6-luna", "gpt-5.6-luna",
               "muse-spark-1.3-contributor", "muse-spark-1.2-contributor"),
    messages=("minimax-m3", "minimax-m2.7", "minimax-m2.5", "qwen3.8-max", "qwen3.8-flash",
              "qwen3.7-max", "qwen3.7-plus", "qwen3.6-plus"),
)

OPENCODE_ZEN_MODEL_TRANSPORTS = _endpoint_transports(
    responses=("gpt-6-astra", "gpt-6-sol", "gpt-6-luna", "gpt-5.6-sol", "gpt-5.6-terra",
               "gpt-5.6-luna", "gpt-5.5", "gpt-5.5-pro", "gpt-5.4", "gpt-5.4-pro",
               "gpt-5.4-mini", "gpt-5.4-nano", "gpt-5.3-codex", "gpt-5.3-codex-spark",
               "gpt-5.2", "gpt-5.2-codex", "gpt-5.1", "gpt-5.1-codex", "gpt-5.1-codex-max",
               "gpt-5.1-codex-mini", "gpt-5", "gpt-5-codex", "gpt-5-nano",
               "grok-4.7", "grok-4.6", "grok-4.5", "grok-build-0.1",
               "muse-spark-1.3", "muse-spark-1.2", "muse-spark-1.3-contributor-free"),
    messages=("claude-fable-5-1", "claude-fable-5", "claude-opus-5-5", "claude-opus-5",
              "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-opus-4-5",
              "claude-sonnet-5", "claude-sonnet-4-6", "claude-sonnet-4-5", "claude-haiku-4-5",
              "qwen3.8-flash", "qwen3.7-max", "qwen3.7-plus", "qwen3.6-plus", "qwen3.5-plus"),
)


# A preset is only offered with a documented compatible endpoint.  Gateways or
# self-hosted services without one still get a recognisable entry, but require
# the owner to enter their own URL rather than silently targeting a guessed host.
PRESETS = (
    preset("openai", "OpenAI", "直接接続", "openai", "https://api.openai.com/v1"),
    preset("anthropic", "Anthropic", "直接接続", "anthropic", "https://api.anthropic.com"),
    preset("gemini", "Google Gemini", "直接接続", "gemini", "https://generativelanguage.googleapis.com"),
    preset("openrouter", "OpenRouter", "ゲートウェイ", "openrouter", "https://openrouter.ai/api/v1"),
    preset("ollama", "Ollama", "ローカル", "ollama", "http://host.docker.internal:11434/v1", False, True),
    preset("lmstudio", "LM Studio", "ローカル", "lmstudio", "http://host.docker.internal:1234/v1", False, True),
    preset("groq", "Groq", "OpenAI互換", "compatible", "https://api.groq.com/openai/v1"),
    preset("xai", "xAI", "OpenAI互換", "compatible", "https://api.x.ai/v1"),
    preset("mistral", "Mistral AI", "OpenAI互換", "compatible", "https://api.mistral.ai/v1"),
    preset("deepseek", "DeepSeek", "OpenAI互換", "compatible", "https://api.deepseek.com/v1"),
    preset("together", "Together AI", "OpenAI互換", "compatible", "https://api.together.xyz/v1"),
    preset("fireworks", "Fireworks AI", "OpenAI互換", "compatible", "https://api.fireworks.ai/inference/v1"),
    preset("perplexity", "Perplexity", "OpenAI互換", "compatible", "https://api.perplexity.ai"),
    preset("cerebras", "Cerebras", "OpenAI互換", "compatible", "https://api.cerebras.ai/v1"),
    preset("sambanova", "SambaNova", "OpenAI互換", "compatible", "https://api.sambanova.ai/v1"),
    preset("nvidia-nim", "NVIDIA NIM", "OpenAI互換", "compatible", "https://integrate.api.nvidia.com/v1"),
    preset("ai21", "AI21", "OpenAI互換", "compatible", "https://api.ai21.com/studio/v1"),
    preset("cohere", "Cohere", "OpenAI互換", "compatible", "https://api.cohere.com/compatibility/v1"),
    preset("huggingface", "Hugging Face", "OpenAI互換", "compatible", "https://router.huggingface.co/v1"),
    preset("cloudflare", "Cloudflare Workers AI", "OpenAI互換", "compatible", extra_config_schema=(
        {"key": "account_id", "label": "Cloudflare Account ID", "required": True},)),
    preset("github-models", "GitHub Models", "OpenAI互換", "compatible", "https://models.github.ai/inference"),
    preset("vercel-ai-gateway", "Vercel AI Gateway", "ゲートウェイ", "compatible", "https://ai-gateway.vercel.sh/v1"),
    preset("litellm", "LiteLLM", "ゲートウェイ", "compatible"),
    preset("portkey", "Portkey", "ゲートウェイ", "compatible"),
    preset("helicone", "Helicone", "ゲートウェイ", "compatible", "https://ai-gateway.helicone.ai/v1"),
    preset("deepinfra", "DeepInfra", "OpenAI互換", "compatible", "https://api.deepinfra.com/v1/openai"),
    preset("nebius", "Nebius AI Studio", "OpenAI互換", "compatible", "https://api.studio.nebius.ai/v1"),
    preset("novita", "Novita AI", "OpenAI互換", "compatible", "https://api.novita.ai/openai/v1"),
    preset("chutes", "Chutes", "OpenAI互換", "compatible", "https://llm.chutes.ai/v1"),
    preset("featherless", "Featherless AI", "OpenAI互換", "compatible", "https://api.featherless.ai/v1"),
    preset("siliconflow", "SiliconFlow", "OpenAI互換", "compatible", "https://api.siliconflow.cn/v1"),
    preset("modelscope", "ModelScope", "OpenAI互換", "compatible"),
    preset("alibaba-model-studio", "Alibaba Cloud Model Studio", "OpenAI互換", "compatible", "https://dashscope.aliyuncs.com/compatible-mode/v1"),
    preset("moonshot", "Moonshot AI", "OpenAI互換", "compatible", "https://api.moonshot.ai/v1"),
    preset("zhipu", "Zhipu AI", "OpenAI互換", "compatible", "https://open.bigmodel.cn/api/paas/v4"),
    preset("minimax", "MiniMax", "OpenAI互換", "compatible", "https://api.minimax.io/v1"),
    preset("baidu-qianfan", "Baidu Qianfan", "OpenAI互換", "compatible"),
    preset("tencent-hunyuan", "Tencent Hunyuan", "OpenAI互換", "compatible"),
    preset("bytedance-ark", "ByteDance Ark", "OpenAI互換", "compatible", "https://ark.cn-beijing.volces.com/api/v3"),
    preset("01-ai", "01.AI", "OpenAI互換", "compatible", "https://api.lingyiwanwu.com/v1"),
    preset("lambda", "Lambda AI", "OpenAI互換", "compatible"),
    preset("vllm", "vLLM", "ローカル", "compatible", "http://host.docker.internal:8000/v1", False, True),
    preset("localai", "LocalAI", "ローカル", "compatible", "http://host.docker.internal:8080/v1", False, True),
    preset("llama-cpp", "llama.cpp server", "ローカル", "compatible", "http://host.docker.internal:8080/v1", False, True),
    preset("jan", "Jan", "ローカル", "compatible", "http://host.docker.internal:1337/v1", False, True),
    preset("text-generation-webui", "text-generation-webui", "ローカル", "compatible", "http://host.docker.internal:5000/v1", False, True),
    preset("koboldcpp", "KoboldCpp", "ローカル", "compatible", "http://host.docker.internal:5001/v1", False, True),
    preset("elyza", "ELYZA", "国内", "compatible"),
    preset("azure-ai-foundry", "Azure AI Foundry", "クラウド", "compatible", extra_config_schema=(
        {"key": "deployment_name", "label": "Deployment name", "required": True},
        {"key": "api_version", "label": "API version", "required": False},)),
    preset("opencode-zen", "OpenCode Zen", "ゲートウェイ", "compatible", "https://opencode.ai/zen/v1",
           model_transports=OPENCODE_ZEN_MODEL_TRANSPORTS),
    # Go rejects inference without this header (HTTP 400 ``MissingSessionID``).
    # Zen answers 200 without it, so only Go declares one.
    preset("opencode-go", "OpenCode Go", "ゲートウェイ", "compatible", "https://opencode.ai/zen/go/v1",
           model_transports=OPENCODE_GO_MODEL_TRANSPORTS, session_header="x-opencode-session"),
    preset("custom", "カスタム", "カスタム", "compatible"),
)
assert len(PRESETS) == 52
BY_ID = {item["id"]: item for item in PRESETS}
assert all(item["transport_id"] and item["discovery_id"] and item["metadata_resolver_ids"] for item in PRESETS)
assert all(transport in TRANSPORT_IDS
           for item in PRESETS for transport in item["model_transports"].values())


def catalog():
    return deepcopy(PRESETS)


def get_preset(preset_id):
    return BY_ID.get(preset_id)
