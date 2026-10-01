"""Strict verification for Agent mode completion.

The default ``agent_completion_issue`` requires (1) an ``update_plan`` call, (2)
no pending work, (3) a non-empty ``verification`` string, and (4) a successful
read-only check after the last write.  This module layers two extensions:

- Risk-specific checks: ``delete_file`` must be followed by ``files_list``,
  ``write_file`` over an existing file must be preceded by ``read_file``,
  ``run_terminal`` with destructive commands (``rm``, ``mv``, ``git``) must be
  paired with a verification tool that inspects the post-state.
- Phase verification: ``update_plan`` may split the plan into phases
  (``phase:<name>``); each phase must be verified before the next begins.
"""

from __future__ import annotations

import re
from collections.abc import Iterable

# Tools whose persisted tool_id alone is enough to identify them as writes.
# ``_is_write`` checks this set first so a stale or missing ``risk`` field on a
# persisted call record cannot let a destructive tool slip past as a read.
WRITE_TOOLS = {
    "write_file", "edit_file", "delete_file", "run_terminal",
    "create_artifact", "web_clip_save", "knowledge_add", "knowledge_delete",
    "memory_add", "memory_update", "memory_delete", "skill_add", "skill_update",
    "schedule_create", "schedule_update", "schedule_delete",
}

DESTRUCTIVE_TERMINAL_PATTERNS = (
    re.compile(r"\brm\b\s+"),
    re.compile(r"\brmdir\b"),
    re.compile(r"\bmv\b\s+"),
    re.compile(r"\bgit\s+(?:reset|checkout|clean|restore)\b"),
    re.compile(r"\b(?:>|>>|tee)\s+"),
    re.compile(r"\bchmod\b"),
    re.compile(r"\bchown\b"),
    re.compile(r"\bdropdb\b|\bdrop\s+table\b|\btruncate\b"),
)

# Tools that count as a positive post-change verification.
CHECK_TOOLS = {
    "read_file", "files_list", "search_files", "workspace_check", "browser_read",
    "browser_screenshot", "browser_extract", "web_search", "web_fetch",
    "web_fetch_pdf", "knowledge_search", "memory_search", "skill_search",
    "skill_resource", "schedule_list", "process_list",
}

PHASE_PREFIX = "phase:"


def phase_name(plan_step: str) -> str | None:
    """Extract a phase tag from a plan step string.

    A step starting with ``phase:<name>`` is treated as a phase boundary; the
    engine records the verified phase so later phases do not satisfy earlier
    phases' verification requirement.
    """
    if not isinstance(plan_step, str):
        return None
    text = plan_step.strip()
    if text.lower().startswith(PHASE_PREFIX):
        name = text[len(PHASE_PREFIX):].strip()
        return name or None
    return None


def _call_succeeded(call) -> bool:
    if call.status != "completed":
        return False
    result = (call.data or {}).get("result") or {}
    return not (isinstance(result, dict) and (result.get("error") or result.get("ok") is False))


def _is_check(call) -> bool:
    return (call.data or {}).get("tool_id") in CHECK_TOOLS and _call_succeeded(call)


def _is_write(call) -> bool:
    """Whether ``call`` mutates persistent state.

    A call is treated as a write when its ``tool_id`` is in :data:`WRITE_TOOLS`
    or when the explicit ``risk`` field is set to a non-``"read"`` value.  A
    missing ``risk`` field is treated as read because the tool registry marks
    the unsafe tools in ``WRITE_TOOLS`` explicitly; this keeps the strict
    verifier from flagging benign reads whose ``risk`` was not yet persisted.
    """
    data = call.data or {}
    tool_id = data.get("tool_id")
    if tool_id in {"update_plan"}:
        return False
    if tool_id in WRITE_TOOLS:
        return True
    risk = data.get("risk")
    return risk is not None and risk != "read"


def _destructive_terminal(call) -> bool:
    if (call.data or {}).get("tool_id") != "run_terminal":
        return False
    command = ((call.data or {}).get("arguments") or {}).get("command", "")
    if not isinstance(command, str):
        return False
    return any(pattern.search(command) for pattern in DESTRUCTIVE_TERMINAL_PATTERNS)


def _read_preceded(calls: list, write_index: int) -> bool:
    """Return whether the ``write_file`` at ``write_index`` is safe.

    A write is safe when either (a) a prior ``read_file`` on the same path
    succeeded (the agent checked what it is overwriting), or (b) a prior
    ``read_file`` on the same path *errored* — which on a fresh path means the
    file does not exist yet, so this write is a creation, not an overwrite, and
    there is nothing stale to clobber.
    """
    if (calls[write_index].data or {}).get("tool_id") != "write_file":
        return True
    path = ((calls[write_index].data or {}).get("arguments") or {}).get("path")
    if not isinstance(path, str) or not path:
        return True
    window_start = max(0, write_index - 12)
    saw_read = False
    for prior in calls[window_start:write_index]:
        if (prior.data or {}).get("tool_id") != "read_file":
            continue
        prior_path = ((prior.data or {}).get("arguments") or {}).get("path")
        if prior_path != path:
            continue
        saw_read = True
        # A successful read proves the agent inspected the existing content.
        if _call_succeeded(prior):
            return True
    # No read at all on this path within the window → the agent never looked.
    # If a read was attempted and failed, treat the file as new (creation).
    return saw_read


def check_phase_advances(plan_steps: Iterable[str]) -> list[str]:
    """Return the ordered list of distinct phase names in a plan."""
    phases: list[str] = []
    seen: set[str] = set()
    for step in plan_steps or []:
        name = phase_name(step)
        if name and name not in seen:
            seen.add(name)
            phases.append(name)
    return phases


def verify_phases(run, calls: list) -> dict[str, bool]:
    """Return per-phase verification result.

    Combines two sources: explicit ``phase_verification`` recorded by the
    engine as check tools succeed, and a counter-based fallback that marks a
    phase verified when any successful check tool appears in the call log while
    that phase is the most recent unverified one. The fallback keeps existing
    Runs well-behaved even when they were completed before explicit recording
    landed.
    """
    plan = (run.data.get("agent_plan") or {}).get("steps") or []
    plan_phases = check_phase_advances(plan)
    if not plan_phases:
        return {}
    recorded = (run.data.get("agent_plan") or {}).get("phase_verification") or {}
    completed = [call for call in calls if call.status == "completed"]
    phase_progress = {name: bool(recorded.get(name)) for name in plan_phases}
    for call in completed:
        if call.data.get("tool_id") == "update_plan":
            continue
        if _is_write(call):
            continue
        if not _is_check(call):
            continue
        for phase in plan_phases:
            if not phase_progress.get(phase):
                phase_progress[phase] = True
                break
    return phase_progress


def strict_issue(run, calls: list) -> str | None:
    """Return a stricter Agent completion issue message.

    Mirrors ``agent_completion_issue`` for the baseline rules and adds risk-
    specific verification for destructive operations and per-phase evidence.
    Returns the first issue found; collect_issues returns them all.
    """
    if run.data.get("snapshot", {}).get("mode") != "agent":
        return None
    issues = collect_issues(run, calls)
    return issues[0] if issues else None


def collect_issues(run, calls: list) -> list[str]:
    """Return every Agent completion issue, not just the first.

    The verifier used to return on the first violation, which could deadlock an
    Agent that fixed one issue only to be interrupted for the next. Surfacing
    them all lets a single repair turn address everything at once.
    """
    if run.data.get("snapshot", {}).get("mode") != "agent":
        return []
    completed = [call for call in calls if call.status == "completed"]
    plan = run.data.get("agent_plan") or {}
    issues: list[str] = []
    if not completed and not plan:
        return []
    if not plan:
        if all(
            (call.data or {}).get("risk") == "read"
            and not ((call.data or {}).get("result") or {}).get("error")
            and ((call.data or {}).get("result") or {}).get("ok", True)
            for call in completed
        ):
            return []
        issues.append("作業計画と残作業が記録されていません。")
        return issues
    if plan.get("pending"):
        issues.append("未完了の作業があります: " + "、".join(plan["pending"][:5]))
    if not plan.get("verification"):
        issues.append("成果を確認した根拠が記録されていません。")

    write_indices = [
        index for index, call in enumerate(completed)
        if _is_write(call)
    ]
    if write_indices:
        # Most recent write must be paired with a successful check tool that
        # landed strictly after it in the call log.
        last_write = write_indices[-1]
        if not any(
            _is_check(call) for call in completed[last_write + 1:]
        ):
            issues.append("最後の変更後に成功した確認結果がありません。")

    # Risk-specific checks: walk completed calls in order.
    for index, call in enumerate(completed):
        tool_id = (call.data or {}).get("tool_id")
        if tool_id == "delete_file":
            # After a delete, expect files_list or search_files to confirm.
            if not any(
                (other.data or {}).get("tool_id") in {"files_list", "search_files"}
                and _call_succeeded(other)
                for other in completed[index + 1:]
            ):
                issues.append("削除後に files_list または search_files で状態を確認してください。")
        elif tool_id == "write_file":
            if not _read_preceded(completed, index):
                issues.append(
                    "write_file は既存ファイルの上書きです。"
                    "事前に read_file で対象ファイルの内容を確認してください。"
                )
        elif _destructive_terminal(call) and not any(
            _is_check(other) for other in completed[index + 1:]
        ):
            issues.append(
                "破壊的なターミナル操作 (rm / mv / git reset 等) の後に"
                "状態確認の check Tool が必要です。"
            )

    # Phase verification: each phase must have its own evidence.
    phases = check_phase_advances(plan.get("steps") or [])
    if phases:
        verification = verify_phases(run, calls)
        for phase in phases:
            if not verification.get(phase):
                issues.append(f"フェーズ '{phase}' の検証結果が記録されていません。")

    return issues


def verification_proof(run, calls: list) -> dict:
    """Return the verification proof payload stored on the Run.

    Used by the engine and the API to surface the verification trail.
    """
    return {
        "phases": verify_phases(run, calls),
        "checks": [
            {
                "tool_id": (call.data or {}).get("tool_id"),
                "at": call.created_at.isoformat() if call.created_at else None,
            }
            for call in calls
            if _is_check(call)
        ],
    }
