"""Verify SDK-bound payloads and native context without contacting real providers."""

import json
from types import SimpleNamespace as NS

import httpx
import pytest
from anthropic import AsyncAnthropic
from google.genai import types
from mix_agent.providers import adapters
from mix_agent.providers.reasoning import resolve_reasoning
from openai import AsyncOpenAI


def sse(events):
    return "".join(
        ("event: " + e["type"] + "\n" if "type" in e else "")
        + "data: "
        + json.dumps(e)
        + "\n\n"
        for e in events
    )


def completion(delta, finish=None):
    return {
        "id": "r",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "test",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
    }


@pytest.mark.parametrize("mode", ["chat", "thinking"])
@pytest.mark.parametrize(
    "kind,model",
    [
        ("openai", "test"),
        ("openrouter", "test"),
        ("anthropic", "claude-sonnet-4-6"),
        ("anthropic", "claude-sonnet-4-5"),
        ("gemini", "gemini-2.5-flash"),
        ("gemini", "gemini-3-flash-preview"),
    ],
)
async def test_wire_options_and_native_tool_continuation(
    monkeypatch, kind, model, mode
):
    settings = {
        "_resolved_reasoning": resolve_reasoning(
            kind, model, {"reasoning": True}, mode, {}
        )
    }
    captured = []
    history = [{"role": "user", "content": "Find the answer"}]
    tools = [
        {
            "model_name": "web_search",
            "description": "search",
            "input_schema": {
                "type": "object",
                "properties": {"query": {"type": "string"}},
            },
        }
    ]

    def handle(req):
        captured.append(json.loads(req.content))
        if kind == "openai":
            native = [
                {
                    "type": "reasoning",
                    "id": "rs",
                    "summary": [],
                    "encrypted_content": "signed",
                },
                {
                    "type": "function_call",
                    "id": "fc",
                    "call_id": "call",
                    "name": "web_search",
                    "arguments": '{"query":"test"}',
                    "status": "completed",
                },
            ]
            events = [
                {
                    "type": "response.reasoning_summary_text.delta",
                    "delta": "Public summary",
                },
                {
                    "type": "response.completed",
                    "response": {
                        "id": "r",
                        "object": "response",
                        "created_at": 0,
                        "status": "completed",
                        "model": model,
                        "output": native,
                        "usage": None,
                    },
                },
            ]
        elif kind == "openrouter":
            events = [
                completion(
                    {
                        "reasoning_details": [
                            {
                                "index": 0,
                                "type": "reasoning.encrypted",
                                "data": "signed",
                            },
                            {
                                "index": 1,
                                "type": "reasoning.summary",
                                "summary": "Public summary",
                            },
                        ],
                        "reasoning": "PRIVATE RAW",
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call",
                                "type": "function",
                                "function": {
                                    "name": "web_search",
                                    "arguments": '{"query":"test"}',
                                },
                            }
                        ],
                    }
                ),
                completion({}, "tool_calls"),
            ]
        else:
            events = [
                {
                    "type": "message_start",
                    "message": {
                        "id": "r",
                        "type": "message",
                        "role": "assistant",
                        "model": model,
                        "content": [],
                        "stop_reason": None,
                        "stop_sequence": None,
                        "usage": {"input_tokens": 1, "output_tokens": 0},
                    },
                },
                {
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "thinking",
                        "thinking": "",
                        "signature": "",
                    },
                },
                {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "thinking_delta", "thinking": "Public summary"},
                },
                {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "signature_delta", "signature": "signed"},
                },
                {"type": "content_block_stop", "index": 0},
                {
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": {
                        "type": "tool_use",
                        "id": "call",
                        "name": "web_search",
                        "input": {},
                    },
                },
                {
                    "type": "content_block_delta",
                    "index": 1,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": '{"query":"test"}',
                    },
                },
                {"type": "content_block_stop", "index": 1},
                {
                    "type": "message_delta",
                    "delta": {"stop_reason": "tool_use", "stop_sequence": None},
                    "usage": {"output_tokens": 20},
                },
                {"type": "message_stop"},
            ]
        return httpx.Response(
            200, headers={"content-type": "text/event-stream"}, text=sse(events)
        )

    transport = httpx.MockTransport(handle)
    if kind in ("openai", "openrouter"):
        monkeypatch.setattr(
            adapters,
            "AsyncOpenAI",
            lambda **kw: AsyncOpenAI(
                **kw, http_client=httpx.AsyncClient(transport=transport)
            ),
        )
    elif kind == "anthropic":
        monkeypatch.setattr(
            adapters,
            "AsyncAnthropic",
            lambda **kw: AsyncAnthropic(
                **kw, http_client=httpx.AsyncClient(transport=transport)
            ),
        )
    else:

        class GeminiClient:
            async def __aenter__(self):
                return self

            async def __aexit__(self, *args):
                pass

            async def generate_content_stream(self, **kw):
                captured.append(kw)

                async def chunks():
                    yield types.GenerateContentResponse(
                        candidates=[
                            types.Candidate(
                                content=types.Content(
                                    role="model",
                                    parts=[
                                        types.Part(text="Public summary", thought=True),
                                        types.Part(
                                            function_call=types.FunctionCall(
                                                id="call",
                                                name="web_search",
                                                args={"query": "test"},
                                            ),
                                            thought_signature=b"signed",
                                        ),
                                    ],
                                ),
                                finish_reason="STOP",
                            )
                        ]
                    )

                return chunks()

            @property
            def models(self):
                return self

        monkeypatch.setattr(
            adapters.genai, "Client", lambda **kw: NS(aio=GeminiClient())
        )
    adapter = adapters.Adapter(
        {"kind": kind, "base_url": "https://provider.example"}, "test-key"
    )
    events = [e async for e in adapter.stream(model, history, tools, mode, settings)]
    assert [e["text"] for e in events if e["kind"] == "reasoning"] == ["Public summary"]
    response = next(e for e in events if e["kind"] == "response")
    assert response["tool_calls"][0]["name"] == "web_search"
    history += [
        response["message"],
        {"role": "tool", "call_id": "call", "name": "web_search", "content": "found"},
    ]
    _ = [e async for e in adapter.stream(model, history, tools, mode, settings)]
    first, second = captured
    if kind == "openai":
        assert first["reasoning"]["effort"] == ("low" if mode == "chat" else "medium")
        assert second["input"][1]["encrypted_content"] == "signed"
        assert second["input"][-1]["type"] == "function_call_output"
    elif kind == "openrouter":
        assert first["reasoning"]["enabled"] is True
        assert first["provider"]["require_parameters"] is True
        assert second["messages"][1]["reasoning_details"][0]["data"] == "signed"
        assert "PRIVATE RAW" not in json.dumps(
            [e for e in events if e["kind"] != "response"]
        )
    elif kind == "anthropic":
        if model.endswith("4-6"):
            assert first["thinking"]["type"] == "adaptive"
            assert first["output_config"]["effort"] == (
                "low" if mode == "chat" else "high"
            )
        else:
            assert first["thinking"] == {"type": "enabled", "budget_tokens": 1024}
        assert second["messages"][1]["content"][0]["signature"] == "signed"
    else:
        if model.startswith("gemini-2.5"):
            assert first["config"]["thinking_config"]["thinking_budget"] == (
                -1 if mode == "chat" else 1024
            )
        else:
            assert first["config"]["thinking_config"]["thinking_level"] == (
                "low" if mode == "chat" else "high"
            )
        assert second["contents"][1].parts[1].thought_signature == b"signed"


@pytest.mark.parametrize(
    "kind", ["openai", "openrouter", "compatible", "ollama", "lmstudio"]
)
async def test_no_unverified_reasoning_parameters(monkeypatch, kind):
    captured = []

    def handle(req):
        captured.append(json.loads(req.content))
        if kind == "openai":
            events = [
                {
                    "type": "response.completed",
                    "response": {
                        "id": "r",
                        "object": "response",
                        "created_at": 0,
                        "status": "completed",
                        "model": "test",
                        "output": [],
                        "usage": None,
                    },
                }
            ]
        else:
            events = [completion({"content": "hello"}, "stop")]
        return httpx.Response(
            200, headers={"content-type": "text/event-stream"}, text=sse(events)
        )

    monkeypatch.setattr(
        adapters,
        "AsyncOpenAI",
        lambda **kw: AsyncOpenAI(
            **kw, http_client=httpx.AsyncClient(transport=httpx.MockTransport(handle))
        ),
    )
    settings = {"_resolved_reasoning": resolve_reasoning(kind, "test", {}, "chat", {})}
    _ = [
        e
        async for e in adapters.Adapter(
            {"kind": kind, "base_url": "https://example.com"}, "test"
        ).stream("test", [], [], "chat", settings)
    ]
    assert "reasoning" not in captured[0]
    assert "reasoning_effort" not in captured[0]
    assert "tools" not in captured[0]
