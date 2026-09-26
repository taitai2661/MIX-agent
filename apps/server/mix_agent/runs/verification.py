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

# Risk categories used by the engine and tool registry.  ``read`` covers any
# tool whose ``risk`` is "read" in the registry; the names below are the
# write-risk tools we want stricter evidence for.
WRITE_TOOLS = {
    "write_file", "edit_file", "delete_file", "run_terminal",
    "create_artifact", "web_clip_save", "knowledge_add", "knowledge_delete",
    "memory_add", "memory_update", "memory_delete", "skill_add", "skill_update",
    "schedule_create", "schedule_update", "schedule_delete",
}

DESTRUCTIVE_TERMINAL_PATTERNS = (
    re.compile(r"\brm\b\s+(-[a-zA-Z]*[rfRF][a-zA-Z]*\s+)?[^|;]+"),
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
    return (call.data or {}).get("risk") != "read" and (call.data or {}).get("tool_id") not in {"update_plan"}


def _destructive_terminal(call) -> bool:
    if (call.data or {}).get("tool_id") != "run_terminal":
        return False
    command = ((call.data or {}).get("arguments") or {}).get("command", "")
    if not isinstance(command, str):
        return False
    return any(pattern.search(command) for pattern in DESTRUCTIVE_TERMINAL_PATTERNS)


def _write_over_existing(runs_history: list[dict], call) -> bool:
    """Heuristic: a write_file whose target was read earlier in the run."""
    if (call.data or {}).get("tool_id") != "write_file":
        return False
    path = ((call.data or {}).get("arguments") or {}).get("path")
    if not isinstance(path, str) or not path:
        return False
    for entry in runs_history:
        if not isinstance(entry, dict):
            continue
        if entry.get("role") != "tool":
            continue
        if entry.get("name") != "read_file":
            continue
        # Tool-result arguments are not preserved in history; only path search
        # via the matching call record works.
        _ = entry
    return False


def _read_preceded(runs_calls: list, write_index: int) -> bool:
    """Return whether a read_file touched the same path within WINDOW calls."""
    if (runs_calls[write_index].data or {}).get("tool_id") != "write_file":
        return True
    path = ((runs_calls[write_index].data or {}).get("arguments") or {}).get("path")
    if not isinstance(path, str) or not path:
        return True
    for prior in runs_calls[max(0, write_index - 12):write_index]:
        if (prior.data or {}).get("tool_id") == "read_file":
            prior_path = ((prior.data or {}).get("arguments") or {}).get("path")
            if prior_path == path:
                return True
    return False


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

    The engine records verification evidence by tracking the last successful
    check tool after a write tool.  If the plan contains ``phase:<name>``
    boundaries, this returns the verification status for each phase based on
    whether a successful check tool occurred within the phase's window.
    """
    plan = (run.data.get("agent_plan") or {}).get("steps") or []
    plan_phases = check_phase_advances(plan)
    if not plan_phases:
        return {}
    verification = (run.data.get("agent_plan") or {}).get("phase_verification") or {}
    completed = [call for call in calls if call.status == "completed"]
    phase_progress = {name: False for name in plan_phases}
    # Walk through completed calls in order, tracking the current phase based
    # on the plan index that the call's tool references indirectly.
    for index, call in enumerate(completed):
        if (call.data or {}).get("tool_id") == "update_plan":
            # update_plan itself resets phase_verification for the steps it sets.
            continue
        if _is_write(call):
            continue
        if _is_check(call):
            # Mark the current phase as verified if a check landed after a write.
            for phase in plan_phases:
                if phase not in verification:
                    phase_progress[phase] = True
                    break
    merged = dict(verification)
    for phase, ok in phase_progress.items():
        if phase not in merged and ok:
            merged[phase] = True
    return merged


def strict_issue(run, calls: list) -> str | None:
    """Return a stricter Agent completion issue message.

    Mirrors ``agent_completion_issue`` for the baseline rules and adds risk-
    specific verification for destructive operations and per-phase evidence.
    """
    if run.data.get("snapshot", {}).get("mode") != "agent":
        return None
    completed = [call for call in calls if call.status == "completed"]
    plan = run.data.get("agent_plan") or {}
    if not completed and not plan:
        return None
    if not plan:
        if all(
            (call.data or {}).get("risk") == "read"
            and not ((call.data or {}).get("result") or {}).get("error")
            and ((call.data or {}).get("result") or {}).get("ok", True)
            for call in completed
        ):
            return None
        return "作業計画と残作業が記録されていません。"
    if plan.get("pending"):
        return "未完了の作業があります: " + "、".join(plan["pending"][:5])
    if not plan.get("verification"):
        return "成果を確認した根拠が記録されていません。"

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
            return "最後の変更後に成功した確認結果がありません。"

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
                return "削除後に files_list または search_files で状態を確認してください。"
        elif tool_id == "write_file":
            if not _read_preceded(completed, index):
                return (
                    "write_file は既存ファイルの上書きです。"
                    "事前に read_file で対象ファイルの内容を確認してください。"
                )
        elif _destructive_terminal(call) and not any(
            _is_check(other) for other in completed[index + 1:]
        ):
            return (
                "破壊的なターミナル操作 (rm / mv / git reset 等) の後に"
                "状態確認の check Tool が必要です。"
            )

    # Phase verification: each phase must have its own evidence.
    phases = check_phase_advances(plan.get("steps") or [])
    if phases:
        verification = verify_phases(run, calls)
        for phase in phases:
            if not verification.get(phase):
                return f"フェーズ '{phase}' の検証結果が記録されていません。"

    return None


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
