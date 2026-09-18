"""Run status state machine.

The PostgreSQL ``runs.status`` column is the single authoritative Run state.
This module is the one place that defines which statuses exist and which
transitions are permitted.  Every Run status change must go through
:func:`transition_run`, which also emits the matching ``status`` Event so the
persisted status and the event trail never diverge.
"""

from __future__ import annotations

RUN_STATES = frozenset(
    {
        "queued",
        "running",
        "waiting_approval",
        "completed",
        "failed",
        "cancelled",
        "interrupted",
    }
)

TERMINAL_STATES = frozenset({"completed", "failed", "cancelled", "interrupted"})

# Legacy alias kept for engine code that predates this module.
TERMINAL = TERMINAL_STATES

NON_TERMINAL_STATES = RUN_STATES - TERMINAL_STATES

# Permitted transitions, derived from the existing engine/API behaviour.
# ``interrupted -> queued`` is the explicit resume path used by the existing
# resume endpoint; ``interrupted -> cancelled`` is what the cancel endpoint
# already allowed.  Terminal states cannot move anywhere else.
TRANSITIONS = {
    "queued": frozenset({"running", "cancelled", "failed"}),
    "running": frozenset({"waiting_approval", "completed", "failed", "cancelled", "interrupted"}),
    "waiting_approval": frozenset({"running", "cancelled", "failed", "interrupted"}),
    "interrupted": frozenset({"queued", "cancelled"}),
    "completed": frozenset(),
    "failed": frozenset(),
    "cancelled": frozenset(),
}


class InvalidRunTransition(ValueError):
    """Raised when a Run status transition is not permitted."""


def is_valid_status(status) -> bool:
    """Return whether ``status`` is a known Run status."""
    return status in RUN_STATES


def can_transition(from_status, to_status) -> bool:
    """Return whether the transition is allowed by the state machine."""
    return (
        to_status in RUN_STATES
        and from_status in RUN_STATES
        and to_status in TRANSITIONS[from_status]
    )


def validate_transition(from_status, to_status, *, expected_from=None):
    """Validate a transition, raising :class:`InvalidRunTransition` on failure."""
    if expected_from is not None:
        expected = {expected_from} if isinstance(expected_from, str) else set(expected_from)
        if from_status not in expected:
            raise InvalidRunTransition(
                f"Run transition {from_status!r} -> {to_status!r} rejected: "
                f"expected current status one of {sorted(expected)}"
            )
    if not can_transition(from_status, to_status):
        raise InvalidRunTransition(f"Invalid Run status transition: {from_status!r} -> {to_status!r}")


def transition_run(db, run, to_status, *, reason=None, expected_from=None) -> bool:
    """Transition ``run`` to ``to_status`` through the state machine.

    The current status is read from the bound ``run`` row (the DB is the
    source of truth).  A transition to the current status is an explicit
    no-op: nothing is changed and no Event is emitted.  A disallowed
    transition raises :class:`InvalidRunTransition`.

    ``reason`` is stored on ``run.data`` and included in the emitted status
    Event.  The status update and the Event are written in the same
    transaction; the caller is responsible for the commit boundary.

    Returns ``True`` when the status changed, ``False`` for an identical
    no-op.
    """
    from_status = run.status
    if from_status == to_status:
        return False
    validate_transition(from_status, to_status, expected_from=expected_from)
    run.status = to_status
    if reason is not None:
        run.data = {**run.data, "reason": reason}
    from mix_agent.runs.engine import emit

    emit(
        db,
        run.id,
        "status",
        {
            "status": to_status,
            "from_status": from_status,
            "reason": reason,
        },
    )
    return True