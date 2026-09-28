"""Run-event kind registry.

The engine emits and the API streams ``run_events.kind`` values.  Each kind
must be listed here so the state machine, the persistence layer, and the
TypeScript event parser stay in lockstep.  Server-side helpers in
``mix_agent.runs.engine`` wrap ``emit`` and ensure every new kind is safe to
stream (no provider secrets, no raw tool payloads).
"""

from __future__ import annotations

# Lifecycle / status
EVENT_STATUS = "status"
# Model interaction
EVENT_TEXT = "text"
EVENT_REASONING = "reasoning"
EVENT_MODEL_STARTED = "model_started"
EVENT_MODEL_SELECTED = "model_selected"
EVENT_MODEL_REROUTED = "model_rerouted"
EVENT_MESSAGE = "message"
# Tools
EVENT_TOOL_STARTED = "tool_started"
EVENT_TOOL_RESULT = "tool_result"
EVENT_PLAN = "plan"
EVENT_APPROVAL = "approval"
EVENT_BROWSER_FRAME = "browser_frame"
# Context and budgets
EVENT_CONTEXT_SUMMARY = "context_summary"
EVENT_CHECKPOINT_SAVED = "checkpoint_saved"
EVENT_BUDGET_EXTENSION_REQUESTED = "budget_extension_requested"
EVENT_BUDGET_EXTENSION_RESOLVED = "budget_extension_resolved"
# Self-correction
EVENT_STAGNATION_DETECTED = "stagnation_detected"
EVENT_VERIFICATION_REQUIRED = "verification_required"
EVENT_PHASE_ADVANCED = "phase_advanced"

ALL_KINDS = frozenset(
    {
        EVENT_STATUS,
        EVENT_TEXT,
        EVENT_REASONING,
        EVENT_MODEL_STARTED,
        EVENT_MODEL_SELECTED,
        EVENT_MODEL_REROUTED,
        EVENT_MESSAGE,
        EVENT_TOOL_STARTED,
        EVENT_TOOL_RESULT,
        EVENT_PLAN,
        EVENT_APPROVAL,
        EVENT_BROWSER_FRAME,
        EVENT_CONTEXT_SUMMARY,
        EVENT_CHECKPOINT_SAVED,
        EVENT_BUDGET_EXTENSION_REQUESTED,
        EVENT_BUDGET_EXTENSION_RESOLVED,
        EVENT_STAGNATION_DETECTED,
        EVENT_VERIFICATION_REQUIRED,
        EVENT_PHASE_ADVANCED,
    }
)


def is_known_kind(kind) -> bool:
    return isinstance(kind, str) and kind in ALL_KINDS
