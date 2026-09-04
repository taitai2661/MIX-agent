"""Provider-specific wire formats stay here; the run engine sees normalized events."""

import httpx
from anthropic import AsyncAnthropic
from google import genai
from google.genai import types
from openai import AsyncOpenAI

from mix_agent.providers.catalog import get_preset
from mix_agent.providers.metadata import context_limit, resolve
from mix_agent.providers.model_roles import chat_capability
from mix_agent.providers.reasoning import request_options, show_summary

DEFAULT_URLS = {
    "openai": "https://api.openai.com/v1",
    "anthropic": "https://api.anthropic.com",
    "gemini": "https://generativelanguage.googleapis.com",
    "openrouter": "https://openrouter.ai/api/v1",
    "ollama": "http://host.docker.internal:11434/v1",
    "lmstudio": "http://host.docker.internal:1234/v1",
    "compatible": "",
}
CAPABILITIES = ("chat", "tools", "vision", "reasoning", "structured_output")


class ProviderConfigurationError(ValueError):
    pass


class ProviderContextLimitError(RuntimeError):
    """A provider explicitly rejected the request for exceeding its context limit."""


def is_retryable_provider_error(exc):
    """Return whether a provider failure is safe to retry on another model."""
    if is_context_limit_error(exc):
        return True
    if isinstance(exc, (httpx.TimeoutException, httpx.NetworkError)):
        return True
    if isinstance(exc, httpx.HTTPStatusError):
        return exc.response.status_code in {408, 425, 429, 500, 502, 503, 504}
    return False


def is_nvidia_nim_function_not_found(provider, exc):
    """Identify NVIDIA's catalog-visible but unavailable hosted-model response.

    NVIDIA NIM may list a model while its hosted inference function is not
    available to the current account.  Keep this narrow so ordinary 404s never
    become Auto retry candidates.
    """
    if provider.get("preset_id") != "nvidia-nim":
        return False
    response = getattr(exc, "response", None)
    status_code = getattr(response, "status_code", None) or getattr(exc, "status_code", None)
    if status_code != 404:
        return False
    details = " ".join(
        str(value) for value in (exc, getattr(exc, "body", ""), getattr(response, "text", "")) if value
    ).casefold()
    return "function" in details and "not found" in details


def is_nvidia_nim_chat_incompatible(provider, model_id, exc):
    """Identify a known NIM special-purpose model rejected by chat completions."""
    if provider.get("preset_id") != "nvidia-nim" or chat_capability("nvidia", model_id) is not False:
        return False
    response = getattr(exc, "response", None)
    status_code = getattr(response, "status_code", None) or getattr(exc, "status_code", None)
    return status_code == 400


def is_context_limit_error(exc):
    if isinstance(exc, ProviderContextLimitError):
        return True
    text = " ".join(str(value) for value in (exc, getattr(exc, "body", ""))).lower()
    return any(marker in text for marker in (
        "context length", "context window", "context limit", "maximum context",
        "too many tokens", "input is too long", "prompt is too long",
    ))


class Adapter:
    def __init__(self, provider, key):
        self.provider = provider
        self.key = key
        self.kind = provider["kind"]
        self.url = provider.get("base_url") or DEFAULT_URLS[self.kind]
        self.preset = get_preset(provider.get("preset_id"))
        # Records created before presets retain a deliberate legacy mapping.
        if not self.preset:
            self.preset = get_preset({"openai": "openai", "anthropic": "anthropic", "gemini": "gemini",
                                      "openrouter": "openrouter", "ollama": "ollama", "lmstudio": "lmstudio"}.get(self.kind, "custom"))
        if not self.preset:
            raise ValueError("Provider preset has no transport configuration")
        self.transport_id = self.preset["transport_id"]
        self.discovery_id = self.preset["discovery_id"]
        missing = [field["key"] for field in self.preset.get("extra_config_schema", [])
                   if field.get("required") and not provider.get("extra_config", {}).get(field["key"])]
        if missing:
            raise ProviderConfigurationError("missing_extra_config:" + ",".join(missing))

    async def list_models(self):
        headers = {"Authorization": "Bearer " + self.key} if self.key else {}
        url = self.url.rstrip("/") + "/models"
        if self.discovery_id == "anthropic_models":
            headers = {"x-api-key": self.key, "anthropic-version": "2023-06-01"}
            # Accept both Anthropic's account-level URL and the commonly
            # entered `/v1` URL.  The security validation intentionally keeps
            # the path, so blindly appending `/v1` here made model discovery
            # call `/v1/v1/models` after the validation changes.
            root = self.url.rstrip("/")
            url = root + ("/models" if root.endswith("/v1") else "/v1/models")
        elif self.discovery_id == "gemini_models":
            headers = {"x-goog-api-key": self.key}
            root = self.url.rstrip("/")
            url = root + ("/models" if root.endswith("/v1beta") else "/v1beta/models")
        async with httpx.AsyncClient(timeout=20, follow_redirects=False) as client:
            if self.discovery_id == "ollama_tags":
                root = self.url.rstrip("/").removesuffix("/v1")
                response = await client.get(root + "/api/tags", headers=headers)
                response.raise_for_status()
                body = response.json()
                items = []
                for tag in body.get("models", []):
                    model_id = tag.get("name", "")
                    detail = await client.post(root + "/api/show", headers=headers, json={"name": model_id})
                    detail.raise_for_status()
                    details = detail.json()
                    model_info = details.get("model_info", {})
                    context = context_limit(details)
                    if not context and isinstance(model_info, dict):
                        context = next((value for key, value in model_info.items()
                                        if key.endswith(".context_length") and isinstance(value, int)
                                        and not isinstance(value, bool)), None)
                    items.append({"id": model_id, "name": model_id, "context_length": context})
                body = {"data": items}
            else:
                response = await client.get(url, headers=headers)
                response.raise_for_status()
                body = response.json()
        result = []
        for item in body.get("data", body.get("models", [])):
            model_id = item.get("id", item.get("name", "")).removeprefix("models/")
            caps = {k: None for k in CAPABILITIES}
            caps["chat"] = chat_capability(self.kind, model_id)
            parameters = item.get("supported_parameters", [])
            if parameters:
                caps.update(
                    tools="tools" in parameters,
                    reasoning="reasoning" in parameters,
                    structured_output="response_format" in parameters,
                )
            modalities = item.get("architecture", {}).get("input_modalities")
            if modalities is not None:
                caps["vision"] = "image" in modalities
            resolved = await resolve(self.kind, model_id, item, self.preset["metadata_resolver_ids"])
            metadata = resolved["metadata"]
            context = metadata.get("context_window")
            result.append({
                "model_id": model_id, "name": item.get("displayName", item.get("name", model_id)),
                "capabilities": caps, "context_window": context["value"] if context else None,
                "context_source": context["source"] if context else None,
                "context_confidence": context["confidence"] if context else None,
                "source": "provider_api", "overrides": {}, **resolved,
            })
        return result

    async def test_connection(self):
        return {"ok": True, "model_count": len(await self.list_models())}

    def get_capabilities(self, model):
        return {**model.get("capabilities", {}), **model.get("overrides", {})}

    async def stream(self, model, messages, tools, mode, settings):
        method = {
            "openai_responses": self._openai,
            "openai_compatible": self._compatible,
            "anthropic_messages": self._anthropic,
            "gemini_generate_content": self._gemini,
            "ollama": self._ollama,
            "lmstudio": self._lmstudio,
        }.get(self.transport_id)
        if method is None:
            raise ValueError(f"Unsupported transport: {self.transport_id}")
        try:
            async for event in method(model, messages, tools, mode, settings):
                yield event
        except Exception as exc:
            if is_context_limit_error(exc):
                raise ProviderContextLimitError("Provider rejected the request because its context limit was exceeded") from exc
            raise

    async def _ollama(self, model, messages, tools, mode, settings):
        """Dedicated slot: Ollama's model discovery is native while its chat endpoint is compatible."""
        async for event in self._compatible(model, messages, tools, mode, settings):
            yield event

    async def _lmstudio(self, model, messages, tools, mode, settings):
        """Dedicated slot for future LM Studio extensions without implicit fallback."""
        async for event in self._compatible(model, messages, tools, mode, settings):
            yield event

    async def probe_tools(self, model):
        """Verify tool-call wire support without exposing or running a real tool."""
        probe = {
            "model_name": "mix_tool_probe",
            "description": "Internal compatibility check. Call this function now.",
            "input_schema": {"type": "object", "properties": {}, "additionalProperties": False},
        }
        settings = {
            "max_output_tokens": 64,
            "_tool_probe": True,
            "_resolved_reasoning": {"policy": "off", "request": {}, "summary": False},
        }
        async for event in self.stream(
            model,
            [{"role": "user", "content": "Call mix_tool_probe now. Do not answer with text."}],
            [probe],
            "chat",
            settings,
        ):
            if event["kind"] == "response":
                return any(call.get("name") == "mix_tool_probe" for call in event.get("tool_calls", []))
        return False

    async def _openai(self, model, messages, tools, mode, settings):
        inputs, instructions = [], ""
        for m in messages:
            if m["role"] == "system":
                instructions = m["content"]
            elif m["role"] == "tool":
                inputs.append(
                    {"type": "function_call_output", "call_id": m["call_id"], "output": m["content"]}
                )
            elif m.get("native"):
                inputs.extend(m["native"])
            else:
                content = m["content"]
                if m.get("images"):
                    content = [{"type": "input_text", "text": content}] + [
                        {"type": "input_image", "image_url": x} for x in m["images"]
                    ]
                inputs.append({"role": m["role"], "content": content})
        args = dict(
            model=model,
            input=inputs,
            instructions=instructions,
            stream=True,
            store=False,
            max_output_tokens=settings.get("max_output_tokens", 4096),
            tools=[
                {
                    "type": "function",
                    "name": t["model_name"],
                    "description": t["description"],
                    "parameters": t["input_schema"],
                }
                for t in tools
            ],
        )
        options = request_options(settings)
        if options is not None:
            args.update(options)
            if not tools:
                args.pop("tools", None)
        elif mode != "chat":
            args.update(
                reasoning={"summary": "auto", "effort": settings.get("reasoning_effort", "medium")},
                include=["reasoning.encrypted_content"],
            )
        if settings.get("_tool_probe"):
            args["tool_choice"] = {"type": "function", "name": "mix_tool_probe"}
        async with AsyncOpenAI(api_key=self.key, base_url=self.url, max_retries=0) as client:
            stream = await client.responses.create(**args)
            async for event in stream:
                if event.type == "response.output_text.delta":
                    yield {"kind": "text", "text": event.delta}
                elif event.type == "response.reasoning_summary_text.delta" and show_summary(mode, settings):
                    yield {"kind": "reasoning", "text": event.delta}
                elif event.type == "response.completed":
                    response = event.response
                    native = [x.model_dump(mode="json", exclude_none=True) for x in response.output]
                    calls = [
                        {"id": x.call_id, "name": x.name, "arguments": x.arguments}
                        for x in response.output
                        if x.type == "function_call"
                    ]
                    yield {
                        "kind": "response",
                        "message": {"role": "assistant", "content": response.output_text, "native": native},
                        "tool_calls": calls,
                        "usage": response.usage.model_dump() if response.usage else {},
                    }
                elif event.type in ("response.failed", "response.incomplete", "error"):
                    raise RuntimeError("Provider stopped before completing the response")

    async def _compatible(self, model, messages, tools, mode, settings):
        wire = []
        for m in messages:
            if m["role"] == "tool":
                wire.append({"role": "tool", "tool_call_id": m["call_id"], "content": m["content"]})
            elif m.get("native"):
                wire.append(m["native"])
            else:
                content = m["content"]
                if m.get("images"):
                    content = [{"type": "text", "text": content}] + [
                        {"type": "image_url", "image_url": {"url": x}} for x in m["images"]
                    ]
                wire.append({"role": m["role"], "content": content})
        args = dict(
            model=model, messages=wire, stream=True, max_tokens=settings.get("max_output_tokens", 4096)
        )
        if tools:
            args["tools"] = [
                {
                    "type": "function",
                    "function": {
                        "name": t["model_name"],
                        "description": t["description"],
                        "parameters": t["input_schema"],
                    },
                }
                for t in tools
            ]
        if settings.get("temperature") is not None:
            args["temperature"] = settings["temperature"]
        args.update(request_options(settings) or {})
        if settings.get("_tool_probe"):
            args["tool_choice"] = {"type": "function", "function": {"name": "mix_tool_probe"}}
        content, calls, finished = "", {}, False
        reasoning_details, raw_reasoning = {}, ""
        async with AsyncOpenAI(api_key=self.key or "local", base_url=self.url, max_retries=0) as client:
            stream = await client.chat.completions.create(**args)
            async for chunk in stream:
                if not chunk.choices:
                    continue
                choice = chunk.choices[0]
                delta = choice.delta
                if delta.content:
                    content += delta.content
                    yield {"kind": "text", "text": delta.content}
                # Only explicit summary fields, never vendor raw chain-of-thought fields.
                summary = getattr(delta, "reasoning_summary", None)
                if summary and show_summary(mode, settings):
                    yield {"kind": "reasoning", "text": summary}
                if self.kind == "openrouter":
                    # Keep signed/encrypted blocks for the next tool turn; only
                    # explicitly identified summaries may become UI events.
                    raw_reasoning += getattr(delta, "reasoning", None) or ""
                    for detail in getattr(delta, "reasoning_details", None) or []:
                        index = detail.get("index", 0)
                        accumulated = reasoning_details.setdefault(index, {})
                        for field, value in detail.items():
                            if value is not None:
                                if field in ("text", "summary", "data", "signature"):
                                    accumulated[field] = accumulated.get(field, "") + value
                                else:
                                    accumulated[field] = value
                        if (
                            accumulated.get("type") == "reasoning.summary"
                            and show_summary(mode, settings)
                            and detail.get("summary")
                        ):
                            yield {"kind": "reasoning", "text": detail["summary"]}
                for call in delta.tool_calls or []:
                    c = calls.setdefault(call.index, {"id": "", "name": "", "arguments": ""})
                    if call.id:
                        c["id"] = call.id
                    if call.function:
                        c["name"] += call.function.name or ""
                        c["arguments"] += call.function.arguments or ""
                if choice.finish_reason:
                    if choice.finish_reason not in ("stop", "tool_calls"):
                        raise RuntimeError("Provider output limit or refusal")
                    finished = True
        if not finished:
            raise RuntimeError("Provider stream ended unexpectedly")
        native = {"role": "assistant", "content": content or None}
        if reasoning_details:
            native["reasoning_details"] = list(reasoning_details.values())
        elif raw_reasoning:
            native["reasoning"] = raw_reasoning
        if calls:
            native["tool_calls"] = [
                {
                    "id": c["id"],
                    "type": "function",
                    "function": {"name": c["name"], "arguments": c["arguments"]},
                }
                for c in calls.values()
            ]
        yield {
            "kind": "response",
            "message": {"role": "assistant", "content": content, "native": native},
            "tool_calls": list(calls.values()),
        }

    async def _anthropic(self, model, messages, tools, mode, settings):
        wire, system = [], ""
        for m in messages:
            if m["role"] == "system":
                system = m["content"]
                continue
            if m["role"] == "tool":
                entry = {
                    "role": "user",
                    "content": [
                        {"type": "tool_result", "tool_use_id": m["call_id"], "content": m["content"]}
                    ],
                }
            else:
                content = m.get("native") or [{"type": "text", "text": m["content"] or " "}]
                for image in m.get("images", []):
                    mime, encoded = image[5:].split(";base64,", 1)
                    content.append(
                        {"type": "image", "source": {"type": "base64", "media_type": mime, "data": encoded}}
                    )
                entry = {"role": m["role"], "content": content}
            if wire and wire[-1]["role"] == entry["role"]:
                wire[-1]["content"].extend(entry["content"])
            else:
                wire.append(entry)
        args = dict(
            model=model,
            system=system,
            messages=wire,
            max_tokens=settings.get("max_output_tokens", 4096),
            tools=[
                {"name": t["model_name"], "description": t["description"], "input_schema": t["input_schema"]}
                for t in tools
            ],
        )
        options = request_options(settings)
        if options is not None:
            args.update(options)
            if not tools:
                args.pop("tools", None)
        elif mode != "chat":
            args["thinking"] = {"type": "enabled", "budget_tokens": min(1024, args["max_tokens"] - 1)}
        if settings.get("_tool_probe"):
            args["tool_choice"] = {"type": "tool", "name": "mix_tool_probe"}
        async with AsyncAnthropic(api_key=self.key, base_url=self.url, max_retries=0) as client:
            async with client.messages.stream(**args) as stream:
                async for event in stream:
                    if event.type == "content_block_delta":
                        if event.delta.type == "text_delta":
                            yield {"kind": "text", "text": event.delta.text}
                        elif event.delta.type == "thinking_delta" and show_summary(mode, settings):
                            yield {"kind": "reasoning", "text": event.delta.thinking}
                final = await stream.get_final_message()
        if final.stop_reason not in ("end_turn", "tool_use", "stop_sequence"):
            raise RuntimeError("Provider stopped before completion")
        native = [x.model_dump(mode="json", exclude_none=True) for x in final.content]
        yield {
            "kind": "response",
            "message": {
                "role": "assistant",
                "content": "".join(x.text for x in final.content if x.type == "text"),
                "native": native,
            },
            "tool_calls": [
                {"id": x.id, "name": x.name, "arguments": x.input}
                for x in final.content
                if x.type == "tool_use"
            ],
            "usage": final.usage.model_dump(),
        }

    async def _gemini(self, model, messages, tools, mode, settings):
        wire, system = [], ""
        for m in messages:
            if m["role"] == "system":
                system = m["content"]
                continue
            if m["role"] == "tool":
                parts = [{"function_response": {"name": m["name"], "response": {"result": m["content"]}}}]
            else:
                parts = m.get("native") or [{"text": m["content"] or " "}]
                for image in m.get("images", []):
                    mime, encoded = image[5:].split(";base64,", 1)
                    parts.append({"inline_data": {"mime_type": mime, "data": encoded}})
            wire.append(
                types.Content.model_validate(
                    {"role": "model" if m["role"] == "assistant" else "user", "parts": parts}
                )
            )
        cfg = {"system_instruction": system, "max_output_tokens": settings.get("max_output_tokens", 4096)}
        if tools:
            cfg["tools"] = [
                {
                    "function_declarations": [
                        {
                            "name": t["model_name"],
                            "description": t["description"],
                            "parameters_json_schema": t["input_schema"],
                        }
                        for t in tools
                    ]
                }
            ]
        options = request_options(settings)
        if options is not None:
            cfg.update(options)
        elif mode != "chat":
            cfg["thinking_config"] = {"include_thoughts": True}
        if settings.get("_tool_probe"):
            cfg["tool_config"] = {
                "function_calling_config": {
                    "mode": "ANY",
                    "allowed_function_names": ["mix_tool_probe"],
                }
            }
        native, calls, text, finished = [], [], "", False
        async with genai.Client(api_key=self.key, http_options={"base_url": self.url}).aio as client:
            async for chunk in await client.models.generate_content_stream(
                model=model, contents=wire, config=cfg
            ):
                for candidate in chunk.candidates or []:
                    if candidate.finish_reason:
                        if str(candidate.finish_reason) not in ("FinishReason.STOP", "STOP"):
                            raise RuntimeError("Provider stopped before completion")
                        finished = True
                    for part in candidate.content.parts if candidate.content else []:
                        native.append(part.model_dump(mode="json", exclude_none=True))
                        if part.text:
                            if part.thought:
                                if show_summary(mode, settings):
                                    yield {"kind": "reasoning", "text": part.text}
                            else:
                                text += part.text
                                yield {"kind": "text", "text": part.text}
                        if part.function_call:
                            calls.append(
                                {
                                    "id": part.function_call.id or f"gemini_{len(calls)}",
                                    "name": part.function_call.name,
                                    "arguments": dict(part.function_call.args or {}),
                                }
                            )
        if not finished:
            raise RuntimeError("Provider stream ended unexpectedly")
        yield {
            "kind": "response",
            "message": {"role": "assistant", "content": text, "native": native},
            "tool_calls": calls,
        }
