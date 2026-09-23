"""The public provider catalog; wire protocols remain a small, audited set."""

from copy import deepcopy

KIND_VALUES = ("openai", "anthropic", "gemini", "openrouter", "ollama", "lmstudio", "compatible")
CUSTOM_KINDS = ("compatible", "anthropic", "gemini")


def preset(id, name, category, kind, url="", api_key=True, private=False, *, transport_id=None,
           discovery_id=None, extra_config_schema=()):
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
            "metadata_resolver_ids": ("provider_api", "official_catalog", "models_dev", "builtin"),
            "extra_config_schema": list(extra_config_schema)}


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
    preset("custom", "カスタム", "カスタム", "compatible"),
)
assert len(PRESETS) == 50
BY_ID = {item["id"]: item for item in PRESETS}
assert all(item["transport_id"] and item["discovery_id"] and item["metadata_resolver_ids"] for item in PRESETS)


def catalog():
    return deepcopy(PRESETS)


def get_preset(preset_id):
    return BY_ID.get(preset_id)
