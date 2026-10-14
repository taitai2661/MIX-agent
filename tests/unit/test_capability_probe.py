"""Capability probe verdicts, precedence, and staleness for Auto routing."""
from datetime import UTC, datetime, timedelta

from mix_agent import config
from mix_agent.providers import capability_probe


def record(status="supported", checked_at=None, **extra):
    return {
        "probe_version": config.TOOL_PROBE_VERSION,
        "status": status,
        "checked_at": (checked_at or datetime.now(UTC)).isoformat(),
        **extra,
    }


class Stub:
    def __init__(self, **outcomes):
        self.outcomes = outcomes
        self.calls = []

    async def _run(self, name, model):
        self.calls.append(name)
        outcome = self.outcomes.get(name, False)
        if isinstance(outcome, Exception):
            raise outcome
        return outcome

    async def probe_tools(self, model):
        return await self._run("tools", model)

    async def probe_vision(self, model):
        return await self._run("vision", model)

    async def probe_reasoning(self, model, resolved):
        return await self._run("reasoning", model)


def test_manual_override_and_provider_data_beat_a_probe():
    data = {
        "capabilities": {"tools": False, "vision": None, "reasoning": True},
        "overrides": {"vision": True},
        "tool_probe": record(status="supported", vision="unsupported", reasoning="unsupported"),
    }
    caps = capability_probe.capabilities(data)
    assert caps["tools"] is False
    assert caps["vision"] is True
    assert caps["reasoning"] is True


def test_probe_fills_only_unknown_capabilities():
    data = {
        "model_id": "test",
        "capabilities": {"tools": None, "vision": False, "reasoning": None},
        "tool_probe": record(status="supported", vision="supported", reasoning="unsupported"),
    }
    caps = capability_probe.capabilities(data)
    assert caps["tools"] is True
    assert caps["vision"] is False
    assert caps["reasoning"] is False


def test_probe_needed_respects_overrides_metadata_and_version():
    assert capability_probe.probe_needed({}, "tools") is True
    assert not capability_probe.probe_needed({"capabilities": {"tools": True}}, "tools")
    assert not capability_probe.probe_needed({"overrides": {"tools": False}}, "tools")
    # An old probe version is stale even when it looks conclusive.
    stale = {"tool_probe": {"probe_version": config.TOOL_PROBE_VERSION - 1, "status": "supported"}}
    assert capability_probe.probe_needed(stale, "tools") is True
    # Reasoning without a verified control protocol is never probed.
    assert not capability_probe.probe_needed({"model_id": "llama-70b"}, "reasoning", "compatible")
    assert capability_probe.probe_needed({"model_id": "gpt-4.1"}, "reasoning", "openai")


def test_known_and_unknown_verdicts_expire_at_different_ages():
    now = datetime.now(UTC)
    fresh = record(checked_at=now - timedelta(days=1))
    assert not capability_probe.probe_needed({"tool_probe": fresh, "model_id": "x"}, "tools")
    old = record(checked_at=now - timedelta(days=config.PROBE_MAX_AGE.days + 1))
    assert capability_probe.probe_needed({"tool_probe": old, "model_id": "x"}, "tools")

    recent_unknown = record(status="unknown", checked_at=now - timedelta(minutes=1))
    assert not capability_probe.probe_needed({"tool_probe": recent_unknown, "model_id": "x"}, "tools")
    stale_unknown = record(status="unknown", checked_at=now - config.PROBE_UNKNOWN_RETRY - timedelta(minutes=1))
    assert capability_probe.probe_needed({"tool_probe": stale_unknown, "model_id": "x"}, "tools")


async def test_probe_merges_verdicts_and_classifies_failures():
    stub = Stub(tools=True, vision=False, reasoning=TimeoutError())
    original = {"model_id": "gpt-4.1", "capabilities": {}, "metadata_overrides": {"keep": True}}
    result = await capability_probe.probe(
        stub, original, ["tools", "vision", "reasoning"], "automatic", "openai"
    )
    stored = result["tool_probe"]
    assert stored["probe_version"] == config.TOOL_PROBE_VERSION
    assert stored["source"] == "automatic"
    assert stored["status"] == "supported"
    assert stored["vision"] == "unsupported"
    assert stored["reasoning"] == "unknown"
    assert stored["failures"] == {"reasoning": "TimeoutError"}
    # The probe record is the only key added; the rest of the model is preserved.
    assert result["metadata_overrides"] == {"keep": True}
    assert stub.calls == ["tools", "vision", "reasoning"]


async def test_probe_preserves_previous_capability_verdicts():
    stub = Stub(tools=False)
    data = {"model_id": "x", "tool_probe": record(status="supported", vision="supported")}
    result = await capability_probe.probe(stub, data, ["tools"], "automatic")
    assert result["tool_probe"]["status"] == "unsupported"
    assert result["tool_probe"]["vision"] == "supported"
