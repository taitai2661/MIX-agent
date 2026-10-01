"""Tests for stagnation detection and strict verification."""

from types import SimpleNamespace

from mix_agent.runs import stagnation
from mix_agent.runs import verification
from mix_agent.runs.engine import agent_completion_issue


def _run(plan=None, mode="agent"):
    return SimpleNamespace(data={"snapshot": {"mode": mode}, "agent_plan": plan})


def _call(tool_id, *, risk="read", result=None, status="completed"):
    return SimpleNamespace(
        status=status,
        data={
            "tool_id": tool_id,
            "risk": risk,
            "result": result or {"ok": True},
        },
    )


def test_stagnation_detects_repeated_tool_failures():
    run = SimpleNamespace(
        id="r1",
        data={"task_state": {"pending": [], "plan": ["phase:a"], "completed": []}},
    )
    calls = [
        _call("read_file", risk="read", result={"error": "fail"}),
        _call("read_file", risk="read", result={"error": "fail"}),
        _call("read_file", risk="read", result={"error": "fail"}),
        _call("read_file", risk="read", result={"error": "fail"}),
    ]
    rows = list(reversed(calls))  # type: ignore
    # The detector reads from the DB, so feed an in-memory shim.
    class _DB:
        def scalar(self, _stmt):
            return None

        def scalars(self, _stmt):
            class _Iter:
                def __iter__(self):
                    return iter(rows)

            return _Iter()

    findings = stagnation.detect_stagnation(_DB(), run)
    codes = {f["code"] for f in findings}
    assert "tool_repeated_failure" in codes


def test_stagnation_detects_stale_plan():
    run = SimpleNamespace(
        id="r2",
        data={"task_state": {"pending": ["check"], "plan": ["phase:a"], "completed": []}},
    )
    # Build 8 calls so PLAN_STEP_LIMIT (6) is exceeded since the last update_plan.
    calls = [
        _call("read_file", risk="read", result={"ok": True}),
        _call("read_file", risk="read", result={"ok": True}),
        _call("read_file", risk="read", result={"ok": True}),
        _call("read_file", risk="read", result={"ok": True}),
        _call("read_file", risk="read", result={"ok": True}),
        _call("read_file", risk="read", result={"ok": True}),
        _call("read_file", risk="read", result={"ok": True}),
        _call("read_file", risk="read", result={"ok": True}),
    ]

    class _DB:
        def scalar(self, _stmt):
            return None

        def scalars(self, _stmt):
            class _Iter:
                def __iter__(self):
                    return iter(calls)

            return _Iter()

    findings = stagnation.detect_stagnation(_DB(), run)
    codes = {f["code"] for f in findings}
    assert "stale_plan" in codes


def test_stagnation_detects_provider_unstable():
    run = SimpleNamespace(id="r3", data={"task_state": {}})
    calls = [
        _call("web_search", risk="read", result={"error": {"code": "provider_5xx"}}),
        _call("web_search", risk="read", result={"error": {"code": "provider_5xx"}}),
        _call("web_search", risk="read", result={"error": {"code": "provider_5xx"}}),
        _call("web_search", risk="read", result={"error": {"code": "provider_5xx"}}),
    ]

    class _DB:
        def scalar(self, _stmt):
            return None

        def scalars(self, _stmt):
            class _Iter:
                def __iter__(self):
                    return iter(calls)

            return _Iter()

    findings = stagnation.detect_stagnation(_DB(), run)
    codes = {f["code"] for f in findings}
    assert "provider_unstable" in codes


def test_strict_verification_requires_post_delete_check():
    run = _run(plan={"steps": ["phase:a"], "pending": [], "verification": "ok"})
    calls = [
        _call("delete_file", risk="write", result={"ok": True}),
        # A successful read check satisfies the post-write requirement but does
        # not satisfy the delete-specific follow-up.
        _call("read_file", risk="read", result={"ok": True}),
    ]
    issue = verification.strict_issue(run, calls)
    assert issue is not None
    assert "files_list" in issue


def test_strict_verification_requires_read_before_overwrite():
    run = _run(plan={"steps": ["phase:a"], "pending": [], "verification": "ok"})
    calls = [
        _call("write_file", risk="write", result={"ok": True}, status="completed"),
        _call("read_file", risk="read", result={"ok": True}),
    ]
    # Need a prior read_file call to satisfy the precondition.
    calls.insert(0, _call("read_file", risk="read", result={"ok": True}))
    assert verification.strict_issue(run, calls) is None


def test_strict_verification_requires_phase_evidence():
    run = _run(plan={"steps": ["phase:setup", "phase:verify"], "pending": [], "verification": "ok"})
    calls = [
        _call("write_file", risk="write", result={"ok": True}),
        _call("read_file", risk="read", result={"ok": True}),
    ]
    issue = verification.strict_issue(run, calls)
    # The phase 'verify' is extracted from "phase:verify".
    assert "verify" in (issue or "")


def test_agent_completion_delegates_to_strict_when_enabled():
    run = _run(plan={"steps": ["phase:a"], "pending": [], "verification": "ok"})
    run.data["snapshot"] = {"mode": "agent", "policy": {"strict_verification": True}}
    calls = [
        _call("delete_file", risk="write", result={"ok": True}),
    ]
    # The legacy agent_completion_issue now defers to strict_issue when the
    # policy enables it; it must still flag the missing post-delete check.
    assert agent_completion_issue(run, calls) is not None
