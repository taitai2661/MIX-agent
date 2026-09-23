"""Unit tests for the Run status state machine (no database required)."""

import pytest
from mix_agent.runs.state import (
    RUN_STATES,
    TERMINAL_STATES,
    TRANSITIONS,
    InvalidRunTransition,
    can_transition,
    is_valid_status,
    validate_transition,
)


def test_run_states_are_defined_in_one_place():
    assert RUN_STATES == {
        "queued",
        "running",
        "waiting_approval",
        "completed",
        "failed",
        "cancelled",
        "interrupted",
    }


def test_terminal_states():
    assert TERMINAL_STATES == {
        "completed",
        "failed",
        "cancelled",
        "interrupted",
    }


@pytest.mark.parametrize(
    ("from_status", "to_status"),
    [
        ("queued", "running"),
        ("queued", "cancelled"),
        ("queued", "failed"),
        ("running", "waiting_approval"),
        ("running", "completed"),
        ("running", "failed"),
        ("running", "cancelled"),
        ("running", "interrupted"),
        ("waiting_approval", "running"),
        ("waiting_approval", "cancelled"),
        ("waiting_approval", "failed"),
        ("waiting_approval", "interrupted"),
        ("interrupted", "queued"),
        ("interrupted", "cancelled"),
    ],
)
def test_allowed_transitions(from_status, to_status):
    assert can_transition(from_status, to_status)
    validate_transition(from_status, to_status)  # must not raise


@pytest.mark.parametrize(
    ("from_status", "to_status"),
    [
        ("completed", "running"),
        ("failed", "running"),
        ("cancelled", "running"),
        ("interrupted", "running"),
        ("queued", "completed"),
        ("running", "queued"),
        ("completed", "interrupted"),
        ("failed", "completed"),
        ("cancelled", "failed"),
        ("interrupted", "completed"),
    ],
)
def test_rejected_transitions(from_status, to_status):
    assert not can_transition(from_status, to_status)
    with pytest.raises(InvalidRunTransition):
        validate_transition(from_status, to_status)


def test_terminal_states_cannot_leave_or_move_among_themselves():
    for terminal in sorted(TERMINAL_STATES):
        for target in sorted(RUN_STATES):
            if terminal == "interrupted" and target in ("queued", "cancelled"):
                continue
            if target == terminal:
                continue
            assert not can_transition(terminal, target), f"{terminal} -> {target}"


def test_unknown_status_is_rejected():
    assert not is_valid_status("paused")
    assert not is_valid_status("")
    assert not is_valid_status(None)
    assert is_valid_status("running")
    with pytest.raises(InvalidRunTransition):
        validate_transition("queued", "paused")
    with pytest.raises(InvalidRunTransition):
        validate_transition("paused", "running")
    with pytest.raises(InvalidRunTransition):
        validate_transition("paused", "paused")


def test_every_state_is_covered_by_transition_table():
    assert set(TRANSITIONS) == set(RUN_STATES)


def test_transition_targets_are_known_statuses():
    for targets in TRANSITIONS.values():
        assert targets <= RUN_STATES


def test_expected_from_mismatch_is_rejected():
    validate_transition("queued", "running", expected_from="queued")
    validate_transition("running", "waiting_approval", expected_from="running")
    with pytest.raises(InvalidRunTransition):
        validate_transition("running", "waiting_approval", expected_from="queued")


def test_validate_transition_accepts_collection_of_expected_from():
    validate_transition("waiting_approval", "running", expected_from=("queued", "waiting_approval"))


def test_state_round_trip_constants_match_schema_literal():
    # Runs.status values must stay in sync with the API schema and the DB
    # CHECK constraint string.  A mismatch here is a real drift bug.
    import mix_agent.api.schemas

    body = mix_agent.api.schemas.RunStatus
    members = set(body.__args__)
    assert members == set(RUN_STATES)
    assert not (members - set(RUN_STATES))