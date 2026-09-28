"""Two-pass tool-call probe: forced choice first, unforced retry second."""
from mix_agent.providers.adapters import Adapter


def response_with_calls(*names):
    return {"kind": "response", "tool_calls": [{"id": "c1", "name": name, "arguments": "{}"} for name in names]}


def text_then_stop():
    return [
        {"kind": "text", "text": "OK, calling now."},
        {"kind": "response", "tool_calls": []},
    ]


class Stub:
    def __init__(self, behaviors):
        self.behaviors = behaviors
        self.calls = []

    async def stream(self, model, messages, tools, mode, settings):
        forced = bool(settings.get("_tool_probe"))
        self.calls.append({"forced": forced, "model": model, "tools": [t["model_name"] for t in tools]})
        for event in self.behaviors.get(forced, ()) or ():
            yield event


async def probe(behaviors):
    stub = Stub(behaviors)
    result = await Adapter.probe_tools(stub, "model-x")
    return result, stub


async def test_forced_probe_supported_in_one_call():
    result, stub = await probe({True: [response_with_calls("mix_tool_probe")]})
    assert result is True
    assert len(stub.calls) == 1
    assert stub.calls[0]["forced"] is True


async def test_ignored_forced_choice_falls_back_to_unforced():
    # NVIDIA NIM-style: the endpoint ignores the forced tool_choice and answers
    # with an empty stop turn, but calls the tool when the choice is left open.
    behaviors = {True: text_then_stop(), False: [response_with_calls("mix_tool_probe")]}
    result, stub = await probe(behaviors)
    assert result is True
    assert [c["forced"] for c in stub.calls] == [True, False]
    assert all(c["tools"] == ["mix_tool_probe"] for c in stub.calls)


async def test_text_only_probe_is_unsupported():
    result, stub = await probe({True: text_then_stop(), False: text_then_stop()})
    assert result is False
    assert len(stub.calls) == 2


async def test_unforced_turn_that_answers_text_still_supported():
    behaviors = {
        True: text_then_stop(),
        False: [{"kind": "text", "text": "calling"}, response_with_calls("mix_tool_probe")],
    }
    result, _ = await probe(behaviors)
    assert result is True


async def test_vision_probe_accepts_a_text_response():
    stub = Stub({False: [{"kind": "response", "message": {"role": "assistant", "content": "ok"}}]})
    assert await Adapter.probe_vision(stub, "model-x") is True
    assert stub.calls[0]["tools"] == []


async def test_vision_probe_rejects_an_empty_response():
    stub = Stub({False: [{"kind": "response", "message": {"role": "assistant", "content": ""}}]})
    assert await Adapter.probe_vision(stub, "model-x") is False


async def test_reasoning_probe_accepts_a_completed_response():
    stub = Stub({False: [{"kind": "response", "message": {"role": "assistant", "content": "ok"}}]})
    resolved = {"policy": "required", "control": "openai", "request": {}, "summary": False}
    assert await Adapter.probe_reasoning(stub, "model-x", resolved) is True