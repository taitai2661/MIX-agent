"""Detect agent stagnation patterns and propose self-repair guidance.

Stagnation is any signal that the agent is not making real progress: repeated
failures, unchanged plans, no read/write activity, or provider instability.
Each detector returns a tuple of (code, message) so the engine can attach it
to a user-role history entry and emit a ``stagnation_detected`` event.
"""

from __future__ import annotations

from collections import defaultdict
from collections.abc import Iterable

from sqlalchemy import select

from mix_agent.db.models import ToolCall

# Number of recent steps inspected for stagnation patterns.
WINDOW = 12
# A single tool failure with a new tool/args is healthy; this is the threshold
# at which repeated failures of the same tool become a stagnation signal.
SAME_TOOL_FAILURE_LIMIT = 4
# Read-only steps without a plan update start to look like the agent is stuck.
READ_ONLY_LIMIT = 8
# A pending plan that has not been updated for this many steps is stale.
PLAN_STEP_LIMIT = 6
# A provider that has surfaced transient errors this many times should trigger
# a routing nudge rather than another retry.
PROVIDER_ERROR_LIMIT = 3

# Tool IDs considered "read" for stagnation purposes (no state change).
_READ_TOOLS = {
    "web_search", "web_fetch", "web_fetch_pdf", "files_list", "read_file",
    "search_files", "browser_read", "browser_screenshot", "browser_extract",
    "workspace_check", "knowledge_search", "memory_search", "skill_search",
    "skill_resource", "process_list",
}


def _tool_call_rows(db, run_id: str) -> list[ToolCall]:
    return list(
        db.scalars(
            select(ToolCall)
            .where(ToolCall.run_id == run_id)
            .order_by(ToolCall.created_at.asc())
        )
    )


def _recent(rows: list[ToolCall], window: int) -> list[ToolCall]:
    if len(rows) <= window:
        return rows
    return rows[-window:]


def _is_failed(call: ToolCall) -> bool:
    if call.status == "failed":
        return True
    result = (call.data or {}).get("result")
    return isinstance(result, dict) and bool(result.get("error"))


def _is_provider_transient(call: ToolCall) -> bool:
    result = (call.data or {}).get("result")
    if not isinstance(result, dict):
        return False
    error = result.get("error") or {}
    code = (error.get("code") if isinstance(error, dict) else None) or ""
    return code in {
        "provider_5xx", "timeout", "incomplete",
        "rate_limit", "tool_timeout",
    }


def detect_stagnation(db, run) -> list[dict]:
    """Return a list of stagnation findings for the current Run state.

    Each finding has ``code``, ``message``, ``severity`` ("info" or "warn"),
    and optional ``suggested_tools`` so the engine can recommend Skill search
    or plan updates.
    """
    findings: list[dict] = []
    rows = _tool_call_rows(db, run.id)
    if not rows:
        return findings
    recent = _recent(rows, WINDOW)

    # 1. Same tool id repeated failures (any args)
    failures_by_tool: dict[str, int] = defaultdict(int)
    for call in recent:
        if not _is_failed(call):
            continue
        tool_id = (call.data or {}).get("tool_id") or ""
        if tool_id in {"update_plan"}:
            continue
        failures_by_tool[tool_id] += 1
    for tool_id, count in failures_by_tool.items():
        if count >= SAME_TOOL_FAILURE_LIMIT:
            findings.append({
                "code": "tool_repeated_failure",
                "severity": "warn",
                "tool_id": tool_id,
                "count": count,
                "message": (
                    f"同じTool '{tool_id}' の失敗が直近{count}回続いています。"
                    "別のToolや参照方法で同じ情報を取得できるか検討し、"
                    "Skill検索で類似作業の手順を確認してください。"
                ),
                "suggested_tools": ["skill_search", "memory_search"],
            })

    # 2. Read-only doldrums: many read tool calls without write/update activity
    read_only = sum(
        1 for call in recent
        if (call.data or {}).get("tool_id") in _READ_TOOLS
    )
    if read_only >= READ_ONLY_LIMIT and not any(
        (call.data or {}).get("tool_id") not in _READ_TOOLS
        and (call.data or {}).get("risk") != "read"
        for call in recent[-READ_ONLY_LIMIT:]
    ):
        findings.append({
            "code": "no_progress",
            "severity": "warn",
            "count": read_only,
            "message": (
                f"直近{read_only}回のTool呼び出しが読み取りのみで、"
                "状態を変更する操作や検証がありません。"
                "update_planで現在の進捗を反映し、次の具体的な作業に進んでください。"
            ),
            "suggested_tools": ["update_plan"],
        })

    # 3. Stale plan: pending list has not been touched for PLAN_STEP_LIMIT steps
    task_state = run.data.get("task_state") or {}
    pending = task_state.get("pending") or []
    plan = task_state.get("plan") or []
    if pending and plan:
        # Approximate step count since the last plan update by counting
        # update_plan + write tool calls in the full history.
        since_update = 0
        for call in reversed(rows):
            if (call.data or {}).get("tool_id") == "update_plan":
                break
            since_update += 1
        if since_update >= PLAN_STEP_LIMIT and len(pending) >= 1:
            findings.append({
                "code": "stale_plan",
                "severity": "info",
                "count": since_update,
                "message": (
                    f"update_planが{PLAN_STEP_LIMIT}ステップ以上更新されていません。"
                    "保留中の作業と完了済み作業を進捗に合わせて整理してください。"
                ),
                "suggested_tools": ["update_plan"],
            })

    # 4. Provider instability
    transient = sum(1 for call in recent if _is_provider_transient(call))
    if transient >= PROVIDER_ERROR_LIMIT:
        findings.append({
            "code": "provider_unstable",
            "severity": "warn",
            "count": transient,
            "message": (
                f"Providerの一時的な失敗が直近{transient}回発生しています。"
                "設定済みの代替ProviderがあるならAutoに切り替えを、"
                "なければ少し時間を置いてから再開することを検討してください。"
            ),
            "suggested_tools": [],
        })

    return findings


def repair_history_message(findings: Iterable[dict]) -> str:
    """Render a single user-role history entry from one or more findings."""
    items = list(findings)
    if not items:
        return ""
    lines = ["Stagnation signals detected. Take a corrective action before continuing:"]
    for finding in items:
        lines.append("- " + finding["message"])
    lines.append(
        "Pick the next concrete step that makes real progress "
        "(read, write, or verify), update the plan, and continue."
    )
    return "\n".join(lines)


def suggested_tools(findings: Iterable[dict]) -> list[str]:
    seen: list[str] = []
    for finding in findings:
        for tool_id in finding.get("suggested_tools") or []:
            if tool_id not in seen:
                seen.append(tool_id)
    return seen
