"""Automatic, privacy-minimal capability confirmation for Auto routing.

Tool Calling is the capability Auto depends on most, but Vision and Reasoning
change selection the same way.  This module keeps the small, fixed verdict set
(supported / unsupported / unknown) for those capabilities, decides when a
stored verdict is stale, and runs the provider probes through an adapter.

No probe executes a real tool and no provider response body is retained: only
the wire-level verdict and a stable error class name are persisted.
"""

from __future__ import annotations

import base64
from datetime import UTC, datetime

from mix_agent import config
from mix_agent.providers.reasoning import reasoning_control, resolve_reasoning

TRACKED = ("tools", "vision", "reasoning")
# Reasoning is only auto-probeable where the provider request is strongly typed
# and rejects an unsupported control.  OpenRouter's free-form extra_body is not.
_REASONING_CONTROLS = frozenset({
    "openai", "anthropic_adaptive", "anthropic_budget", "gemini_budget", "gemini_level",
})
# ``tools`` keeps its historical ``status`` key so older readers stay valid.
_VERDICT_KEYS = {"tools": "status"}
_VERDICTS = {"supported": True, "unsupported": False}
# A 1x1 transparent PNG: enough to exercise the image wire format cheaply.
_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
)
VISION_DATA_URL = "data:image/png;base64," + base64.b64encode(_PNG).decode()


def _key(capability):
    return _VERDICT_KEYS.get(capability, capability)


def _parse(value):
    if not isinstance(value, str):
        return None
    try:
        parsed = datetime.fromisoformat(value)
    except ValueError:
        return None
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=UTC)


def record(data):
    """Return the stored probe record when it matches the current version."""
    stored = data.get("tool_probe") if isinstance(data, dict) else None
    if not isinstance(stored, dict) or stored.get("probe_version") != config.TOOL_PROBE_VERSION:
        return {}
    return stored


def _verdict(stored, capability):
    value = stored.get(_key(capability))
    return _VERDICTS.get(value) if isinstance(value, str) else None


def capabilities(data):
    """Effective capability map: manual override > provider data > probe."""
    result = dict(data.get("capabilities") or {})
    result.update(data.get("overrides") or {})
    stored = record(data)
    for capability in TRACKED:
        if result.get(capability) is None:
            verdict = _verdict(stored, capability)
            if verdict is not None:
                result[capability] = verdict
    return result


def capability(data, name):
    return capabilities(data).get(name)


def _age_exceeded(stored, current, maximum):
    checked = _parse(stored.get("checked_at"))
    if checked is None:
        return True
    current = current or datetime.now(UTC)
    return current - checked >= maximum


def probe_needed(data, capability, kind=None, current=None):
    """Whether an automatic probe should (re)check ``capability``.

    Manual overrides and authoritative provider metadata always win, so only
    genuinely unknown verdicts are confirmed.  Reasoning is only probeable for
    providers that expose a verified control protocol.
    """
    if capability not in TRACKED:
        return False
    if (data.get("overrides") or {}).get(capability) is not None:
        return False
    if (data.get("capabilities") or {}).get(capability) is not None:
        return False
    if capability == "reasoning" and reasoning_control(
        kind or "", data.get("model_id", "")
    ) not in _REASONING_CONTROLS:
        return False
    stored = record(data)
    if not stored:
        return True
    if stored.get(_key(capability)) not in _VERDICTS:
        return _age_exceeded(stored, current, config.PROBE_UNKNOWN_RETRY)
    return _age_exceeded(stored, current, config.PROBE_MAX_AGE)


def due_capabilities(data, kind=None, current=None):
    return [capability for capability in TRACKED if probe_needed(data, capability, kind, current)]


def merge(data, verdicts, failures, source, current=None):
    """Fold fresh verdicts into a copy of ``data`` without dropping other keys."""
    current = current or datetime.now(UTC)
    stored = dict(data.get("tool_probe") or {})
    stored["probe_version"] = config.TOOL_PROBE_VERSION
    stored["checked_at"] = current.isoformat()
    stored["source"] = source
    merged_failures = dict(stored.get("failures") or {})
    for capability, verdict in verdicts.items():
        stored[_key(capability)] = verdict
        if capability == "tools" and verdict != "unknown":
            stored.pop("failure", None)
    for capability, failure in failures.items():
        merged_failures[capability] = failure
        if capability == "tools":
            stored["failure"] = failure
    for capability in verdicts:
        if capability not in failures:
            merged_failures.pop(capability, None)
    if merged_failures:
        stored["failures"] = merged_failures
    else:
        stored.pop("failures", None)
    return {**data, "tool_probe": stored}


async def probe(adapter, data, requested, source, kind=None, current=None):
    """Run the requested probes and return ``data`` with a merged record.

    Adapter failures are classified into ``unknown`` and never propagate, so a
    single unavailable provider cannot abort a background pass or a request.
    """
    kind = kind or ""
    model_id = data.get("model_id", "")
    verdicts, failures = {}, {}
    for capability in requested:
        try:
            if capability == "tools":
                supported = await adapter.probe_tools(model_id)
            elif capability == "vision":
                supported = await adapter.probe_vision(model_id)
            elif capability == "reasoning":
                settings = resolve_reasoning(kind, model_id, {"reasoning": True}, "thinking", {})
                if settings.get("control") is None:
                    continue
                supported = await adapter.probe_reasoning(model_id, settings)
            else:
                continue
            verdicts[capability] = "supported" if supported else "unsupported"
        except Exception as exc:  # noqa: BLE001 - classified, never leaked
            verdicts[capability] = "unknown"
            failures[capability] = type(exc).__name__
    return merge(data, verdicts, failures, source, current)
