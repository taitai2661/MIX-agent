import asyncio
import json
import re
from collections import defaultdict, deque
from datetime import UTC, datetime, timedelta
from urllib.parse import urlsplit

from sqlalchemy import func, select

from mix_agent import todos as conversation_todos
from mix_agent.auth.security import read_secret
from mix_agent.context import budget as context_budget
from mix_agent.context import head_blocks
from mix_agent.context import task_state as context_task_state
from mix_agent.context import tokens as context_tokens
from mix_agent.context import tools_selector as context_tools
from mix_agent.context.builder import select_recent
from mix_agent.context.references import (
    DEFAULT_TOOL_INLINE_LIMIT,
    tool_envelope_text,
    tool_ref_message,
)
from mix_agent.context.summary import finalize as finalize_summary
from mix_agent.context.summary import merge_prompt as summary_merge_prompt
from mix_agent.context.summary import render as render_summary
from mix_agent.context.types import ContextBudgetError
from mix_agent.db.models import (
    Approval,
    Artifact,
    Conversation,
    Event,
    Message,
    Model,
    Provider,
    Run,
    ScheduledRun,
    Settings,
    ToolCall,
    now,
)
from mix_agent.db.session import SessionLocal
from mix_agent.performance import record as record_performance
from mix_agent.providers.adapters import (
    Adapter,
    ProviderIncompleteResponseError,
    is_nvidia_nim_chat_incompatible,
    is_nvidia_nim_function_not_found,
    is_retryable_provider_error,
)
from mix_agent.providers.reasoning import resolve_reasoning
from mix_agent.reliability import classify_failure, retry_after, usage_scope
from mix_agent.reliability import record as record_reliability
from mix_agent.routing import effective_capabilities, select_auto_model
from mix_agent.usage import record as record_usage
from mix_agent.runs.state import TERMINAL, can_transition, transition_run
from mix_agent.tools.execute import execute, is_user_artifact, runner_request
from mix_agent.tools.registry import call_scope, fingerprint, permission, registry

TASKS = {}
PARALLEL_TOOL_LIMIT = 4
SAME_TOOL_ARGUMENT_LIMIT = 3
# Persisting every provider token makes the database the throughput bottleneck
# for fast models.  This stays well below the interval at which a human can
# perceive streaming updates, while retaining the durable event trail.
STREAM_TEXT_FLUSH_SECONDS = 0.075
_PROVIDER_RATE_LOCK = asyncio.Lock()
_PROVIDER_REQUESTS = defaultdict(deque)
_LAUNCH_SEQUENCE = 0


def _collect_memory_observations(db, run, final_answer: str) -> list[dict]:
    """Build the synchronous Memory Evaluator inputs from a finished Run.

    The Evaluator only fires for items that are *likely* durable knowledge:
    Goals, Decisions, Failures and Experiences that emerged during the Run.
    Plain conversation content is left to the async ``MemoryProcessingJob``
    so we don't pay the synchronous cost on every interactive turn.
    """
    from mix_agent.memory import types as mem_types

    observations: list[dict] = []
    task_state = run.data.get("task_state") or {}
    agent_plan = run.data.get("agent_plan") or {}

    # 1. Goal: lift the structured goal into Working/Task scope so the next
    # Run in the same conversation can recall what the agent was trying to do.
    goal = (task_state.get("goal") or "").strip()
    if goal:
        observations.append({
            "summary": goal[:400],
            "content": goal[:2000],
            "role": mem_types.ROLE_GOAL,
            "scope": mem_types.SCOPE_TASK,
            "source_kind": mem_types.SOURCE_AGENT,
            "verification": mem_types.VERIFICATION_PENDING,
            "entities": [],
            "concepts": [],
            "task_id": run.id,
            "role_metadata": {"status": "in_progress"},
            "explicit_user": False,
        })

    # 2. Constraint items the agent committed to during planning.
    for constraint in task_state.get("constraints") or []:
        text = str(constraint).strip()
        if not text:
            continue
        observations.append({
            "summary": text[:400],
            "content": text[:2000],
            "role": mem_types.ROLE_CONSTRAINT,
            "scope": mem_types.SCOPE_TASK,
            "source_kind": mem_types.SOURCE_AGENT,
            "verification": mem_types.VERIFICATION_PENDING,
            "entities": [],
            "concepts": [],
            "task_id": run.id,
        })

    # 3. Decisions: anything in plan steps or in the explicit verification line.
    verification = (agent_plan.get("verification") or "").strip()
    if verification:
        observations.append({
            "summary": verification[:400],
            "content": verification[:2000],
            "role": mem_types.ROLE_DECISION,
            "scope": mem_types.SCOPE_TASK,
            "source_kind": mem_types.SOURCE_AGENT,
            "verification": mem_types.VERIFICATION_VERIFIED,
            "task_id": run.id,
            "role_metadata": {"reason": verification[:1000], "status": "active"},
        })
    for step in agent_plan.get("steps") or []:
        text = str(step).strip()
        if not text:
            continue
        if text.lower().startswith("phase:"):
            continue
        observations.append({
            "summary": text[:400],
            "content": text[:2000],
            "role": mem_types.ROLE_DECISION,
            "scope": mem_types.SCOPE_TASK,
            "source_kind": mem_types.SOURCE_AGENT,
            "verification": mem_types.VERIFICATION_PENDING,
            "task_id": run.id,
            "role_metadata": {"reason": "", "status": "active"},
        })

    # 4. Failures: tool calls that returned an error during this Run.  These
    # are the agent's most valuable memories - the next Run must not retry
    # the same blind approach.  Read via the caller's DB session so the
    # failure scan sees the same transaction state as the rest of the run.
    seen_failures: set[str] = set()
    from mix_agent.db.models import ToolCall

    try:
        for call in db.scalars(
            select(ToolCall)
            .where(ToolCall.run_id == run.id, ToolCall.status.in_(("completed", "failed")))
            .order_by(ToolCall.created_at.desc())
            .limit(24)
        ):
            data = dict(call.data or {})
            result = data.get("result") or {}
            if not isinstance(result, dict) or not result.get("error"):
                continue
            tool_id = str(data.get("tool_id") or data.get("name") or "tool")
            arguments = data.get("arguments") or {}
            signature = f"{tool_id}:{json.dumps(arguments, sort_keys=True, ensure_ascii=False)[:200]}"
            if signature in seen_failures:
                continue
            seen_failures.add(signature)
            observations.append({
                "summary": f"{tool_id} で失敗",
                "content": json.dumps({
                    "tool": tool_id,
                    "arguments": arguments,
                    "error": result.get("error"),
                }, ensure_ascii=False)[:2000],
                "role": mem_types.ROLE_FAILURE,
                "scope": mem_types.SCOPE_TASK,
                "source_kind": mem_types.SOURCE_TOOL,
                "verification": mem_types.VERIFICATION_VERIFIED,
                "task_id": run.id,
                "role_metadata": {
                    "attempt": tool_id,
                    "outcome": "failed",
                    "reason": str(result.get("error"))[:1000],
                    "lesson": str(agent_plan.get("verification") or "")[:1000] or "同じ引数での再試行を避ける",
                    "related_tool": tool_id,
                    "retry_suggested": False,
                },
            })
    except Exception:  # noqa: BLE001 - failure scan must never abort memory formation
        pass

    # 5. Experience: short summary of what the agent actually achieved, useful
    # when the user comes back tomorrow and asks "did we already do this?".
    if final_answer and final_answer.strip():
        observations.append({
            "summary": final_answer.strip()[:400],
            "content": final_answer.strip()[:2000],
            "role": mem_types.ROLE_EXPERIENCE,
            "scope": mem_types.SCOPE_TASK,
            "source_kind": mem_types.SOURCE_AGENT,
            "verification": mem_types.VERIFICATION_VERIFIED,
            "task_id": run.id,
            "role_metadata": {
                "what": final_answer.strip()[:600],
                "outcome": "completed",
                "follow_up": str(agent_plan.get("pending") or "")[:600],
            },
        })

    return observations


def _next_launch_epoch():
    global _LAUNCH_SEQUENCE
    _LAUNCH_SEQUENCE += 1
    return _LAUNCH_SEQUENCE


def _current_launch_epoch():
    """Return the launch_epoch bound to the running drive task, if any."""
    return getattr(asyncio.current_task(), "launch_epoch", None)


def auto_retry_count(value) -> int:
    """Return a safe retry count for legacy or manually edited saved settings."""
    # SettingsInput rejects malformed values for new writes, but old JSONB rows
    # can still contain null, strings, or booleans.  A malformed persisted value
    # must never make a provider-failure recovery path crash.
    return value if isinstance(value, int) and not isinstance(value, bool) and value >= 0 else 3


async def wait_for_provider_slot(provider):
    """Apply the optional per-process provider requests/minute limit."""
    limit = provider.get("rate_limit_rpm", 0)
    if not isinstance(limit, int) or isinstance(limit, bool) or limit <= 0:
        return
    period = 1 if provider.get("rate_limit_period", "minute") == "second" else 60
    key = provider.get("id") or provider.get("base_url") or provider.get("kind", "unknown")
    while True:
        async with _PROVIDER_RATE_LOCK:
            current = asyncio.get_running_loop().time()
            requests = _PROVIDER_REQUESTS[key]
            while requests and current - requests[0] >= period:
                requests.popleft()
            if len(requests) < limit:
                requests.append(current)
                return
            delay = max(0.05, period - (current - requests[0]))
        await asyncio.sleep(delay)


def is_parallel_safe(tool):
    """Keep user-configured and remote tools serial until the runner can attest safety."""
    return bool(tool and tool.get("source") == "builtin" and tool.get("parallel_safe"))


def output_tokens(usage):
    """Read common provider usage shapes without retaining the full payload."""
    if not isinstance(usage, dict):
        return None
    for key in ("output_tokens", "completion_tokens", "candidates_token_count"):
        value = usage.get(key)
        if isinstance(value, int) and not isinstance(value, bool) and value > 0:
            return value
    return None


def activity_summary(tool_id, arguments):
    """Return the small, safe status payload shown in the conversation UI."""
    labels = {
        "web_search": ("search", "Webを検索中"),
        "web_fetch": ("globe", "Webページを確認中"),
        "files_list": ("file", "ファイルを確認中"),
        "read_file": ("file", "ファイルを読み込み中"),
        "search_files": ("search", "ファイルを検索中"),
        "write_file": ("file", "ファイルを変更中"),
        "edit_file": ("file", "ファイルを変更中"),
        "delete_file": ("file", "ファイルを削除中"),
        "run_terminal": ("terminal", "コマンドを実行中"),
        "workspace_check": ("terminal", "ワークスペースを検証中"),
        "process_list": ("terminal", "実行中の処理を確認中"),
        "process_stop": ("terminal", "処理を停止中"),
        "browser_open": ("globe", "ブラウザでページを開いています"),
        "browser_click": ("globe", "ブラウザを操作中"),
        "browser_type": ("globe", "ブラウザに入力中"),
        "browser_read": ("globe", "ブラウザの内容を確認中"),
        "browser_screenshot": ("globe", "画面を確認中"),
        "memory_search": ("memory", "Memoryを検索中"),
        "memory_add": ("memory", "Memoryを保存中"),
        "memory_update": ("memory", "Memoryを更新中"),
        "memory_delete": ("memory", "Memoryを削除中"),
        "skill_search": ("memory", "Skillを検索中"),
        "skill_add": ("memory", "Skillを保存中"),
        "skill_update": ("memory", "Skillを更新中"),
        "skill_resource": ("memory", "Skill資料を読取中"),
        "update_plan": ("plan", "作業計画を更新中"),
        "schedule_list": ("clock", "定期実行を確認中"),
        "schedule_create": ("clock", "定期実行を作成中"),
        "schedule_update": ("clock", "定期実行を更新中"),
        "schedule_delete": ("clock", "定期実行を削除中"),
        "schedule_run": ("clock", "定期実行を開始中"),
    }
    icon, label = labels.get(tool_id, ("tool", "ツールを実行中"))
    summary = {"icon": icon, "label": label}
    if tool_id == "web_search" and isinstance(arguments.get("query"), str):
        query = arguments["query"].strip()
        if query:
            summary["detail"] = query[:160] + ("…" if len(query) > 160 else "")
    return summary


def activity_result(tool_id, result):
    """Keep only public search source metadata for the compact result row."""
    if tool_id != "web_search" or not isinstance(result, dict):
        return None
    sources = []
    for item in result.get("results", []):
        if not isinstance(item, dict) or not isinstance(item.get("url"), str):
            continue
        parsed = urlsplit(item["url"])
        if parsed.scheme not in ("http", "https") or not parsed.hostname:
            continue
        sources.append({"host": parsed.hostname, "url": item["url"]})
    return {
        "icon": "search",
        "label": f"{len(sources)}件のWebサイトを検索しました",
        "sources": sources[:6],
        "remaining": max(0, len(sources) - 6),
    }


def _reject_tool_call(db, run, provider_call, available, *, error: str, code: str) -> dict:
    """Record a Tool Call the model could not express, and tell the model why.

    The Tool Call row is still persisted so the transcript shows exactly which
    provider call id failed, and a paired tool-role message goes into history so
    the provider protocol stays well formed on the next request. Returns a
    summary the caller can use to decide whether anything runnable happened.
    """
    name = str(provider_call.get("name") or "")[:120]
    known = sorted(t["model_name"] for t in available)[:40]
    if code == "tool_unavailable":
        error += " Available tools: " + (", ".join(known) if known else "(none)")
    call = ToolCall(
        owner_id=run.owner_id,
        run_id=run.id,
        data={
            "tool_id": name or "unknown",
            "tool_version": "",
            "provider_call_id": provider_call.get("id") or "",
            "name": name or "unknown",
            "arguments": {},
            "activity": {"icon": "alert", "label": f"{name or 'Tool'} を呼び出せません"},
            "risk": "external",
            "result": {"error": error, "code": code, "type": code},
        },
    )
    db.add(call)
    db.flush()
    call.status = "completed"
    call.data = {**call.data, "result_activity": activity_result(name or "unknown", {})}
    content = json.dumps(model_tool_result({"error": error, "code": code, "type": code}), ensure_ascii=False)
    update(run, history=[*run.data["history"], {
        "role": "tool",
        "call_id": provider_call.get("id") or "",
        "name": name or "unknown",
        "content": content,
    }])
    emit(db, run.id, "tool_result", {
        "id": call.id,
        "name": call.data["name"],
        "result": {"error": error, "code": code, "type": code},
        "activity": call.data["activity"],
    })
    return {"code": code, "name": name, "available": known, "call_id": call.id}


def model_tool_result(result):
    """Return a safe, explicit outcome envelope for the next model turn."""
    if isinstance(result, dict) and result.get("error"):
        error_type = result.get("type")
        code = result.get("code")
        error = {"code": code if isinstance(code, str) and code.isidentifier() else "tool_failed"}
        if isinstance(error_type, str) and error_type.isidentifier():
            error["type"] = error_type
        messages = {
            "tool_loop_detected": "The same tool and arguments have already been attempted three times. Choose a different action or answer using the available results.",
            "tool_call_limit_reached": "The tool-call limit for this run has been reached. Answer using the available information.",
            "tool_denied": "The tool could not run because permission was not granted.",
            "tool_definition_changed": "The tool definition changed before execution. Review it and make a new call if still needed.",
            "tool_timeout": "The tool did not finish before its time limit.",
        }
        error["message"] = messages.get(error["code"], "The tool execution failed. Choose an appropriate next action or answer from the information already available.")
        return {
            "status": "failed",
            "error": error,
            "next_step": "Choose an appropriate next action or answer from the information already available.",
        }
    return {"status": "succeeded", "result": result}


def tool_attempt_key(tool_id, arguments):
    """Use stable JSON so logically identical object arguments share a retry budget."""
    try:
        return tool_id, json.dumps(arguments, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    except (TypeError, ValueError):
        return tool_id, repr(arguments)


def completed_tool_attempts(db, run_id):
    """Return executed attempts only; pending calls must not consume the retry budget."""
    attempts = defaultdict(int)
    calls = db.scalars(select(ToolCall).where(ToolCall.run_id == run_id, ToolCall.status == "completed"))
    for call in calls:
        if "result" in call.data:
            attempts[tool_attempt_key(call.data["tool_id"], call.data.get("arguments", {}))] += 1
    return attempts


_TRANSITIONAL_ACTION = re.compile(
    r"^(?:(?:これから|まず|では|引き続き|少々)?\s*)?"
    r"(?:確認|調査|検索|調べ|作業|対応|処理|実行|作成|修正|検証|分析|開始|続行|進行|(?:対応を)?進め)"
    r"(?:して(?:いき)?ます|します|を始めます|を続けます)$"
)
_TRANSITIONAL_ENGLISH = re.compile(
    r"^(?:(?:i(?:'ll| will)|let me)\s+)?(?:check|investigate|search|work on|handle|process|run|create|fix|verify|analyze|continue|start)(?:\s+(?:this|it|that|the task))?(?:\s+(?:now|next|first))?$",
    re.IGNORECASE,
)


def is_transitional_only(content):
    """Detect a short progress promise that does not itself satisfy the request."""
    if not isinstance(content, str) or len(content.strip()) > 240:
        return False
    sentences = [
        part.strip(" \t\r\n。.!！")
        for part in re.split(r"[。.!！]+", content.strip())
        if part.strip(" \t\r\n。.!！")
    ]
    acknowledgements = {
        "承知しました", "了解しました", "かしこまりました", "わかりました",
        "understood", "sure", "okay", "ok",
    }
    actions = [sentence for sentence in sentences if sentence.casefold() not in acknowledgements]
    if not actions:
        return False
    return all(
        bool(_TRANSITIONAL_ACTION.fullmatch(sentence) or _TRANSITIONAL_ENGLISH.fullmatch(sentence))
        or sentence in {"少々お待ちください", "しばらくお待ちください"}
        for sentence in actions
    )


def is_user_facing_answer(content):
    """Reject blank, tool-payload-only, and progress-promise-only final turns."""
    if not isinstance(content, str) or not content.strip():
        return False
    if is_transitional_only(content):
        return False
    try:
        payload = json.loads(content)
    except (TypeError, ValueError):
        return True
    return not isinstance(payload, (dict, list))


def tool_attempts_for_fallback(history):
    """Summarize attempted tools without exposing their arguments or result bodies."""
    attempts = []
    for message in history:
        if message.get("role") != "tool":
            continue
        try:
            result = json.loads(message.get("content", "{}"))
        except (TypeError, ValueError):
            result = {}
        state = "失敗" if isinstance(result, dict) and (
            result.get("error") or result.get("status") == "failed"
        ) else "完了"
        name = message.get("name")
        if isinstance(name, str) and name:
            attempts.append(f"{name}（{state}）")
    return "、".join(attempts[-6:]) or "利用可能な情報の確認"


def answer_fallback(history):
    return (
        "回答を作成するために " + tool_attempts_for_fallback(history) + " を試しましたが、"
        "ユーザー向けの本文を生成できませんでした。取得済みの情報だけでは確実な回答にできないため、"
        "知りたい条件や対象をもう少し具体的にして、もう一度依頼してください。"
    )


def agent_completion_issue(run, calls):
    """Require a current plan and post-change evidence before agent completion.

    When the policy enables ``strict_verification``, this delegates to
    :func:`mix_agent.runs.verification.strict_issue` which adds risk-specific
    and phase-specific checks.
    """
    from mix_agent.runs.verification import strict_issue

    if run.data.get("snapshot", {}).get("mode") != "agent":
        return None
    policy = (run.data.get("snapshot") or {}).get("policy") or {}
    if policy.get("strict_verification"):
        strict = strict_issue(run, calls)
        if strict:
            return strict
    completed = [call for call in calls if call.status == "completed"]
    plan = run.data.get("agent_plan") or {}
    if not completed and not plan:
        return None
    if not plan:
        if all(call.data.get("risk") == "read" and not (call.data.get("result") or {}).get("error")
               and (call.data.get("result") or {}).get("ok", True)
               for call in completed):
            return None
        return "作業計画と残作業が記録されていません。"
    if plan.get("pending"):
        return "未完了の作業があります: " + "、".join(plan["pending"][:5])
    if not plan.get("verification"):
        return "成果を確認した根拠が記録されていません。"
    changes = [index for index, call in enumerate(completed)
               if call.data.get("risk") != "read" and call.data.get("tool_id") != "update_plan"]
    if changes:
        check_ids = {"read_file", "files_list", "search_files", "workspace_check", "browser_read", "browser_screenshot",
                     "browser_extract", "web_search", "web_fetch", "web_fetch_pdf", "knowledge_search", "memory_search",
                     "skill_search", "skill_resource", "schedule_list", "process_list"}
        if not any(call.data.get("tool_id") in check_ids and isinstance(call.data.get("result"), dict)
                   and not call.data["result"].get("error") and call.data["result"].get("ok", True)
                   for call in completed[changes[-1] + 1:]):
            return "最後の変更後に成功した確認結果がありません。"
    return None


def agent_completion_issues(run, calls) -> list[str]:
    """Return every Agent completion issue, not just the first.

    Used by the drive loop so a single repair turn can address all violations
    at once, instead of fixing only to be interrupted for the next one.
    """
    from mix_agent.runs.verification import collect_issues

    if run.data.get("snapshot", {}).get("mode") != "agent":
        return []
    policy = (run.data.get("snapshot") or {}).get("policy") or {}
    if policy.get("strict_verification"):
        strict = collect_issues(run, calls)
        if strict:
            return strict
    singular = agent_completion_issue(run, calls)
    return [singular] if singular else []


def agent_interruption_reason(run, reason):
    pending = (run.data.get("agent_plan") or {}).get("pending") or []
    return reason + (" 残作業: " + "、".join(pending[:5]) if pending else "")


def refresh_task_state(head_text, state):
    """Keep the current structured plan in the system head after compaction.

    Rewrites only the Task state block. Every other block — the summary,
    recalled Memory, Skills and Knowledge — is carried over untouched: this
    runs on every step, so a boundary bug here silently blinds long Agent runs
    to the very context they accumulated.
    """
    rendered = context_task_state.render(state)
    if not rendered:
        return head_text
    return head_blocks.with_block(head_text, head_blocks.TASK_STATE, rendered)


def emit(db, run_id, kind, data):
    sequence = (db.scalar(select(func.max(Event.sequence)).where(Event.run_id == run_id)) or 0) + 1
    db.add(Event(run_id=run_id, sequence=sequence, kind=kind, data=data))
    db.flush()
    from mix_agent.wakeups import mark
    mark(db, "run", run_id)


def flush_stream_text(db, run_id, buffered_text) -> bool:
    """Persist a group of streamed text fragments in their original order."""
    if not buffered_text:
        return False
    emit(db, run_id, "text", {"text": "".join(buffered_text)})
    buffered_text.clear()
    db.commit()
    return True


def update(run, **fields):
    run.data = {**run.data, **fields}


def finish(db, run, status, reason=None):
    # Run status changes go through the state machine, which also emits the
    # matching status Event.  A run that already reached a terminal state is
    # never overwritten (e.g. an API cancel racing a drive completion).
    if run.status in TERMINAL:
        return
    transition_run(db, run, status, reason=reason)
    update(run, reason=reason)
    if status in {"failed", "cancelled", "interrupted"} and run.data.get("answer_evaluation", {}).get("status") not in {"provided", "needs_review"}:
        update(run, answer_evaluation={"status": "unanswered", "reason": "最終回答の前に実行が終了しました。"})
    if run.data.get("snapshot", {}).get("policy", {}).get("checkpointing"):
        update(
            run,
            checkpoint={
                "status": status,
                "steps": run.data.get("steps", 0),
                "tool_count": run.data.get("tool_count", 0),
                "finished_at": now().isoformat(),
            },
        )
        if status in {"interrupted", "failed"}:
            try:
                from mix_agent.runs import checkpoints as run_checkpoints
                checkpoint = run_checkpoints.save(db, run, trigger=f"terminal:{status}")
                emit(db, run.id, "checkpoint_saved", {
                    "id": checkpoint.id,
                    "step": checkpoint.step,
                    "trigger": checkpoint.trigger,
                    "tool_count": checkpoint.tool_count,
                })
            except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
                db.rollback()
    scheduled_id = run.data.get("scheduled_run_id")
    if scheduled_id:
        scheduled = db.get(ScheduledRun, scheduled_id)
        if scheduled:
            retry = status == "failed" and int(scheduled.data.get("attempt", 0)) == 0
            scheduled.status = "retrying" if retry else ("completed" if status == "completed" else "failed")
            scheduled.data = {**scheduled.data, "reason": reason, "finished_at": now().isoformat(),
                              **({"attempt": 1, "retry_at": (now() + timedelta(seconds=30)).isoformat()} if retry else {})}
            if retry:
                from mix_agent.wakeups import mark
                mark(db, "scheduler")
            from mix_agent.schedules import notify
            if not retry:
                notify(db, run.owner_id, "schedule.completed" if status == "completed" else "schedule.failed", "定期実行: " + ("完了" if status == "completed" else "失敗"), scheduled)
    db.commit()
    if run.data.get("temporary_mode"):
        from mix_agent.api.routes import purge_temporary_run
        purge_temporary_run(db, run)
        db.commit()


def launch(run_id):
    from mix_agent.storage import backup as backup_module

    # A backup/restore is replacing tables and runner state: never start new
    # work mid-restore. Queued runs are picked up again at the next scheduler
    # tick once ACTIVE clears.
    if backup_module.ACTIVE:
        return
    if run_id in TASKS and not TASKS[run_id].done():
        return
    epoch = _next_launch_epoch()
    # Persist the epoch so a stale task can detect that it no longer owns the
    # run.  Without this, a cancelled drive's finish() can kill a run that was
    # immediately resumed (cancel -> resume redraws a non-terminal status).
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        if run:
            update(run, launch_epoch=epoch)
            db.commit()
    task = asyncio.create_task(drive(run_id))
    task.launch_epoch = epoch
    TASKS[run_id] = task


async def reroute_after_provider_error(db, run, request_key, error, classification, retry_until):
    """Select an unused Auto candidate after a retryable provider failure."""
    routing = run.data.get("auto_routing")
    snapshot = run.data["snapshot"]
    attempts = run.data.get("auto_selection", {}).get("attempts", [])
    failures = [attempt for attempt in attempts if attempt.get("outcome") == "provider_error"]
    retry_limit = auto_retry_count(snapshot.get("auto_retry_count", 3))
    if not routing or len(failures) >= retry_limit:
        return None
    previous_id = snapshot["model_record_id"]
    provider_id = snapshot.get("provider_record_id")
    used_model_ids = {
        previous_id,
        *(attempt["model_record_id"] for attempt in attempts if attempt.get("model_record_id")),
    }
    model, selection = select_auto_model(
        db, run.owner_id, routing["allowed_ids"], routing["content"], routing["mode"],
        routing["artifact_mimes"], routing["tools_required"], routing["context_parts"],
        routing["reserved_output_tokens"], request_key, routing["attachment_bytes"],
        excluded_model_ids=tuple(used_model_ids),
        prefer_other_provider_than=(provider_id if classification in {"rate_limit", "provider_5xx", "timeout", "incomplete"} else None),
        priority=routing.get("priority", "balanced"),
    )
    if not model:
        return None
    provider = db.get(Provider, model.data["provider_id"])
    if not provider:
        return None
    try:
        reasoning = resolve_reasoning(
            provider.data["kind"], model.data["model_id"], effective_capabilities(model.data),
            snapshot["mode"], snapshot.get("model_settings", {}),
        )
    except ValueError:
        return None
    updated_snapshot = {
        **snapshot,
        "model_id": model.data["model_id"],
        "model_record_id": model.id,
        "provider": provider.data,
        "provider_record_id": provider.id,
        "reasoning": reasoning,
    }
    retry_number = len(failures) + 1
    attempts = [*attempts, {
        "model_record_id": previous_id,
        "model_id": snapshot["model_id"],
        "outcome": "provider_error",
        "error_type": type(error).__name__,
        "classification": classification,
    }, {
        "model_record_id": model.id,
        "model_id": model.data["model_id"],
        "outcome": "retry",
        "retry_number": retry_number,
        "retry_limit": retry_limit,
    }]
    selection["attempts"] = attempts
    # A retryable provider rejection produces no model output or tool work, so
    # it must not consume the conversation's execution-step budget.
    update(
        run,
        snapshot=updated_snapshot,
        auto_selection=selection,
        auto_model_history=[*run.data.get("auto_model_history", []), {
            "step": run.data.get("steps", 1), "from_model_record_id": previous_id,
            "to_model_record_id": model.id, "model_id": model.data["model_id"],
        }],
        steps=max(0, run.data.get("steps", 0) - 1),
    )
    emit(db, run.id, "model_rerouted", {
        "from_model_record_id": previous_id,
        "to_model_record_id": model.id,
        "retry_number": retry_number,
        "retry_limit": retry_limit,
        "selection": selection,
    })
    db.commit()
    # Cross-provider alternatives are immediate. When a Retry-After response left
    # only this provider, respect it before making the next distinct-model attempt.
    if retry_until and provider.id == provider_id:
        delay = max(0, (retry_until - now()).total_seconds())
        if delay:
            await asyncio.sleep(delay)
    elif classification in {"provider_5xx", "timeout", "incomplete"} and provider.id == provider_id:
        await asyncio.sleep(min(4, 2 ** max(0, retry_number - 1)))
    return updated_snapshot


def _extract_bad_request_detail(exc):
    body = getattr(exc, "body", None)
    if isinstance(body, dict):
        msg = body.get("message") or (body.get("error", {}).get("message") if isinstance(body.get("error"), dict) else None)
        if msg:
            return f"詳細: {msg} "
    msg = getattr(exc, "message", None)
    if msg and str(msg) != "None":
        return f"詳細: {str(msg)[:200]} "
    return ""


def provider_failure_reason(exc, provider=None):
    """Return a safe, actionable provider failure message without remote details."""
    response = getattr(exc, "response", None)
    status_code = getattr(response, "status_code", None) or getattr(exc, "status_code", None)
    error_name = type(exc).__name__
    if is_nvidia_nim_function_not_found(provider or {}, exc):
        return "NVIDIA NIMでこのアカウントに利用可能な推論機能が見つかりません。NVIDIA側の提供状況またはAPI Keyの利用権限を確認してください。"
    if status_code == 404 or error_name == "NotFoundError":
        return "選択したモデルがProviderに見つかりません。モデル一覧を更新するか、Providerの接続先とモデルIDを確認してください。"
    if error_name == "BadRequestError" or status_code == 400:
        detail = _extract_bad_request_detail(exc)
        return f"Providerがリクエストを拒否しました（400）。{detail}モデルIDとProviderの接続先を確認してください。"
    if status_code in (401, 403) or error_name in {"AuthenticationError", "PermissionDeniedError"}:
        return "Providerの認証または権限が拒否されました。API Keyと利用権限を確認してください。"
    if error_name == "ProviderConfigurationError":
        return "Providerの接続設定が不足しています。Provider設定を確認してください。"
    return "実行に失敗しました（" + error_name + "）。接続設定とモデル対応機能を確認してください。"


def complete_tool_call(db, run, call, current, result):
    """Persist one result in provider-call order after execution has settled."""
    if isinstance(result, dict) and call.data.get("tool_id", "").startswith("browser_"):
        frame_bytes = result.pop("frame_base64", None)
        result.pop("frame_unavailable", None)
        if frame_bytes:
            try:
                from mix_agent.tools.execute import save_artifact as _save_artifact
                import base64

                frame = _save_artifact(db, run.owner_id, base64.b64decode(frame_bytes), "browser-frame.png", "image/png", kind="browser-frame")
                frames = list(run.data.get("browser_frames") or [])
                from urllib.parse import urlsplit, urlunsplit
                address = urlsplit(result.get("url", ""))
                visible_url = urlunsplit((address.scheme, address.netloc, address.path, "", ""))[:2048]
                frames.append({"artifact_id": frame["artifact_id"], "call_id": call.id, "url": visible_url, "tool": call.data["tool_id"]})
                if len(frames) > 30:
                    from mix_agent.db.models import Artifact
                    from mix_agent import config
                    for old in frames[:-30]:
                        old_id = old["artifact_id"]
                        old_row = db.get(Artifact, old_id)
                        if old_row:
                            (config.ARTIFACTS / old_id).unlink(missing_ok=True)
                            db.delete(old_row)
                    frames = frames[-30:]
                update(run, browser_frames=frames)
                emit(db, run.id, "browser_frame", frames[-1])
            except Exception:
                pass  # A screenshot failure must not fail the browser operation.
    call.status = "completed"
    summary = activity_result((current or {}).get("id", call.data["tool_id"]), result)
    call.data = {**call.data, "result": result, **({"result_activity": summary} if summary else {})}
    if call.data.get("tool_id") == "update_plan" and isinstance(result, dict) and not result.get("error"):
        previous = run.data.get("agent_plan") or {}
        agent_plan = {
            "steps": result["steps"],
            "pending": result.get("pending", previous.get("pending", result["steps"])),
            "verification": result.get("verification", previous.get("verification", "")),
        }
        state = context_task_state.validate(run.data.get("task_state"))
        state["plan"] = agent_plan["steps"]
        state["pending"] = agent_plan["pending"]
        update(run, agent_plan=agent_plan, task_state=state)
        # Mirror the checklist onto the conversation so todos outlive the run.
        conversation = db.get(Conversation, run.conversation_id)
        if conversation is not None:
            conversation.data = {**conversation.data, "todos": conversation_todos.derive(agent_plan)}
    elif call.data.get("risk") != "read" and not (isinstance(result, dict) and result.get("error")):
        previous = run.data.get("agent_plan")
        if previous:
            update(run, agent_plan={**previous, "verification": ""})
    _maybe_record_phase_verification(db, run, call, result)
    envelope = model_tool_result(result)
    content = json.dumps(envelope, ensure_ascii=False)
    tool_ref = None
    # Long outputs stay out of persistent history: store raw as an artifact
    # and keep a small summary envelope (tool protocol fields preserved).
    try:
        inline_limit = DEFAULT_TOOL_INLINE_LIMIT
        settings_row = db.get(Settings, "settings") if Settings else None
        if settings_row and isinstance((settings_row.data or {}).get("tool_output_inline_limit"), int):
            inline_limit = max(1000, min(100000, settings_row.data["tool_output_inline_limit"]))
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        inline_limit = DEFAULT_TOOL_INLINE_LIMIT
    is_long, short_summary = tool_envelope_text(content, inline_limit)
    artifact_info = result.get("artifact") if isinstance(result, dict) else None
    if is_long and not (isinstance(artifact_info, dict) and artifact_info.get("artifact_id")):
        try:
            from mix_agent.tools.execute import save_artifact as _save_artifact

            artifact_info = _save_artifact(
                db, run.owner_id, content.encode("utf-8"), "tool-result.json",
                "application/json", kind="context-tool-output",
            )
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            artifact_info = None
    if is_long:
        tool_ref = (artifact_info or {}).get("artifact_id")
        entry = tool_ref_message(
            call.data["provider_call_id"], call.data["name"],
            short_summary[:2000], tool_ref, True,
        )
        history_entry = entry
    else:
        history_entry = {
            "role": "tool",
            "call_id": call.data["provider_call_id"],
            "name": call.data["name"],
            "content": content,
        }
    update(
        run,
        history=[*run.data["history"], history_entry],
    )
    artifact = result.get("artifact") if isinstance(result, dict) else None
    if is_user_artifact(artifact):
        existing = run.data.get("artifacts", [])
        if not any(item.get("artifact_id") == artifact["artifact_id"] for item in existing):
            update(run, artifacts=[*existing, artifact])
    if isinstance(artifact_info, dict) and artifact_info.get("artifact_id"):
        refs = list(run.data.get("tool_refs") or [])
        if artifact_info["artifact_id"] not in refs:
            update(run, tool_refs=[*refs, artifact_info["artifact_id"]])
    event_data = {"id": call.id, "name": call.data["name"], "result": result}
    if summary:
        event_data["activity"] = summary
    emit(db, run.id, "plan" if call.data["name"] == "update_plan" else "tool_result", event_data)
    db.commit()


def _maybe_record_phase_verification(db, run, call, result):
    """Mark the current plan phase as verified when a check tool succeeds.

    Phases are declared with ``phase:<name>`` prefixes in ``update_plan``. A
    phase is "verified" as soon as any read-only inspection tool succeeds while
    that phase is the most recent one in the plan — the Agent demonstrates it
    looked at the post-state before moving on. The recorded
    ``phase_verification`` map is consumed by ``verification.verify_phases``.
    """
    from mix_agent.runs import verification as run_verification

    if call.data.get("tool_id") == "update_plan":
        return
    if not run_verification._is_check(call) or not run_verification._call_succeeded(call):
        return
    plan = run.data.get("agent_plan") or {}
    steps = plan.get("steps") or []
    current = run_verification.check_phase_advances(steps)
    if not current:
        return
    active = current[-1]
    recorded = dict(plan.get("phase_verification") or {})
    if recorded.get(active):
        return
    recorded[active] = True
    update(run, agent_plan={**plan, "phase_verification": recorded})
    emit(db, run.id, "phase_advanced", {"phase": active, "verified": True})

async def execute_prepared_calls(db, run, snapshot, prepared):
    """Run a consecutive, already-authorized batch and preserve its result order.

    Parallel-safe tools run in separate sessions: a single SQLAlchemy session
    shared across coroutines can interleave flushes and lose concurrent updates
    to ``run.data`` (e.g. two tools truncating long output overwrite each
    other's ``tool_refs``). Each worker commits its own writes (artifacts) and
    returns only the reference deltas, which are merged on the caller's
    session before results are finalized in provider-call order.
    """
    for call, current in prepared:
        call.status = "executing"
        emit(db, run.id, "tool_started", {
            "id": call.id,
            "name": current["model_name"],
            "activity": activity_summary(current["id"], call.data["arguments"]),
        })
    db.commit()  # Write-ahead markers forbid replay after a crash.

    async def one(call, current):
        try:
            remaining = max(
                1,
                snapshot.get("max_seconds", 900)
                - (now() - run.created_at.replace(tzinfo=UTC)).total_seconds(),
            )
            with SessionLocal() as worker:
                worker_run = worker.get(Run, run.id)
                result = await asyncio.wait_for(
                    execute(worker, worker_run, current, call.data["arguments"]),
                    timeout=min(120, remaining),
                )
                worker.commit()
                return result, list(worker_run.data.get("tool_refs") or [])
        except asyncio.CancelledError:
            raise
        except Exception as exc:  # noqa: BLE001 - intentionally classified; never leak raw details
            # Do not leak SDK headers, secrets, or remote response bodies.
            return {
                "error": "Tool failed",
                "code": "tool_timeout" if isinstance(exc, TimeoutError) else "tool_failed",
                "type": type(exc).__name__,
            }, None

    batch = await asyncio.gather(*(one(call, current) for call, current in prepared))
    for (call, current), (result, worker_refs) in zip(prepared, batch):
        if worker_refs:
            merged = list(dict.fromkeys([*(run.data.get("tool_refs") or []), *worker_refs]))
            update(run, tool_refs=merged)
        complete_tool_call(db, run, call, current, result)


def _image_data_url(db, owner_id, artifact_id):
    """Load an image artifact as a data URL; return None when missing (degrade)."""
    try:
        row = db.get(Artifact, artifact_id)
        if not row or row.owner_id != owner_id:
            return None
        from mix_agent import config as _config

        raw = (_config.ARTIFACTS / artifact_id).read_bytes()
        import base64 as _b64

        mime = (row.data or {}).get("mime", "image/png")
        return "data:" + mime + ";base64," + _b64.b64encode(raw).decode()
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        return None


def _resolve_history_for_provider(db, run, history):
    """Resolve image_refs at send time; strip ref pointers from transport.

    Missing files degrade to text (never crash); misses are recorded on the run.
    """
    resolved, missing = [], []
    for message in history or []:
        entry = dict(message)
        refs = entry.pop("image_refs", None) or []
        entry.pop("tool_ref", None)
        images = list(entry.get("images") or [])
        # Hoist any legacy inline base64 into refs would have happened at build;
        # here only resolve stored refs.
        for ref_id in refs:
            data_url = _image_data_url(db, run.owner_id, ref_id)
            if data_url:
                images.append(data_url)
            else:
                missing.append(ref_id)
        # Drop legacy inline base64 from transport beyond the newest image message.
        if images:
            entry["images"] = images
        else:
            entry.pop("images", None)
        resolved.append(entry)
    # Keep only the newest image-bearing message with actual images to avoid
    # resending past images every turn.
    last_with_images = max(
        (i for i, m in enumerate(resolved) if m.get("images")), default=None
    )
    if last_with_images is not None:
        for i, m in enumerate(resolved):
            if m.get("images") and i != last_with_images:
                m = dict(m)
                m.pop("images", None)
                resolved[i] = m
    if missing:
        update(run, missing_image_refs=missing)
    return resolved


async def _maybe_compact_context(db, run, snapshot, provider, key):
    """Pre-send budget check with progressive compaction.

    Overflow order: tool raw already ref'd at write time; move old
    conversation to summary; system / current user / task state are kept last.
    Returns provider-ready history (refs resolved).
    """
    from mix_agent.db.models import Model as _Model
    from mix_agent.context import tiered_summary as tiered

    history = run.data.get("history") or []
    # Checkpoint: keep structured task state alive even for legacy runs.
    if not run.data.get("task_state"):
        first_user = next((m.get("content", "") for m in history if m.get("role") == "user"), "")
        update(run, task_state=context_task_state.ensure(None, first_user[:1000]))
    if history and history[0].get("role") == "system":
        head = refresh_task_state(history[0].get("content", ""), run.data["task_state"])
        if head != history[0].get("content"):
            history = [{**history[0], "content": head}, *history[1:]]
            update(run, history=history)
    if run.data.get("summary") is None:
        tiered.install_summary(run.data, "", 0)
    model_record = db.get(_Model, snapshot.get("model_record_id")) if snapshot.get("model_record_id") else None
    model_data = dict((model_record.data if model_record else {}) or {})
    window_info = context_budget.resolve_window(model_data, snapshot)
    snapshot["context_window_info"] = window_info
    # Recompute budget on every step so provider switching re-budgets.
    tool_cost = context_tools.schema_cost(snapshot.get("tools") or [], snapshot.get("model_id", ""))
    total = context_budget.input_budget(window_info, tool_schema_tokens=tool_cost)
    model_id = snapshot.get("model_id", "")
    # total already excludes tool schemas: compare messages-only (no double count).
    estimated = context_tokens.count_messages(history, model_id)
    if estimated <= total:
        _persist_trace(db, run, snapshot, history, estimated, total, tool_cost, [])
        return _resolve_history_for_provider(db, run, history)
    # Evict old conversation (keep head + newest); summarize evicted progressively.
    twelve_pct = max(2000, int(total * 0.12))
    recent_budget = max(1000, total - twelve_pct - tool_cost)
    # Head is history[0] (system block); compact only the conversation tail.
    head, tail = (history[:1], history[1:]) if history else ([], [])
    recent, evicted = select_recent(tail, recent_budget, model_id)
    summarized: list = []
    if evicted:
        previous = (run.data.get("summary") or {}).get("text", "")
        summary_text = ""
        try:
            emit(db, run.id, "context_summary", {"status": "started", "evicted": len(evicted)})
            db.commit()
            summary_input = summary_merge_prompt(previous, evicted)
            async with asyncio.timeout(90):
                async for event in Adapter(provider, key).stream(
                    snapshot["model_id"], summary_input, [], "chat",
                    {"max_output_tokens": 2048, "_session_id": run.conversation_id},
                ):
                    if event["kind"] == "response":
                        summary_text = event["message"]["content"]
            summary_text = finalize_summary(summary_text)
            if summary_text:
                # Tiered summary: rotate the previous active text into the
                # archive so the next compaction can reuse it as evidence.
                new_covered = int((run.data.get("summary") or {}).get("covered_count", 0)) + len(evicted)
                tiered.archive_summary(run.data, summary_text, new_covered)
                summarized = [{"evicted": len(evicted), "tier": 0}]
                emit(db, run.id, "context_summary", {
                    "status": "completed",
                    "summary": summary_text,
                    "archive_rows": len(tiered.current_archive(run.data)),
                })
                # Rewrite head summary line progressively (no full re-summarization).
                head_text = (head[0].get("content") if head else "")
                head_text = _replace_summary_block(head_text, summary_text)
                if head:
                    head = [{**head[0], "content": head_text}]
                db.commit()
        except ContextBudgetError:
            # Re-raise upstream so the caller can decide.  We still want to
            # preserve the existing summary text; no archive rotation.
            emit(db, run.id, "context_summary", {"status": "failed"})
            db.commit()
            raise
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            # A summarizer failure must never cost the user their transcript.
            # Keep run.data["history"] exactly as it was, record why compaction
            # degraded, and let the next compaction opportunity retry. Continuing
            # with an over-budget history is recoverable; silently dropping the
            # evicted tail is not.
            emit(db, run.id, "context_summary", {
                "status": "failed", "evicted": len(evicted), "preserved": True,
            })
            update(run, context_compaction={
                "status": "degraded", "reason": "summary_failed",
                "uncompacted_messages": len(history), "covered_count": 0,
            })
            db.commit()
            raise ContextBudgetError(
                "[history preserved] 要約の生成に失敗したため、古い会話を削除せず Context を"
                "圧縮できませんでした。履歴は保持されています。新しい会話で再開するか、"
                "Summary 対応が安定した Window サイズのモデルへ切り替えてください。"
            ) from None
        if not summary_text:
            emit(db, run.id, "context_summary", {
                "status": "failed", "evicted": len(evicted), "preserved": True,
            })
            update(run, context_compaction={
                "status": "degraded", "reason": "summary_empty",
                "uncompacted_messages": len(history), "covered_count": 0,
            })
            db.commit()
            raise ContextBudgetError(
                "[history preserved] 要約が空のため、古い会話を削除せず Context を圧縮できませんでした"
                "（履歴は保持されています）。新しい会話で再開してください。"
            )
    compacted = [*head, *recent]
    estimated = context_tokens.count_messages(compacted, model_id)
    if estimated > total + 2000:
        raise ContextBudgetError(
            f"context budget exceeded: estimated {estimated} > {total} tokens "
            f"(window {window_info.get('context_window')})"
        )
    update(run, history=compacted)
    _persist_trace(db, run, snapshot, compacted, estimated, total, tool_cost, summarized)
    db.commit()
    return _resolve_history_for_provider(db, run, compacted)


def _replace_summary_block(head_text: str, summary_text: str) -> str:
    """Swap the Prior conversation summary block, keeping every other block.

    An empty summary removes the block. The blocks after the summary (Memory,
    Skills, Knowledge) are the ones an Agent needs most once a run gets long, so
    this must never truncate them.
    """
    return head_blocks.with_block(head_text, head_blocks.PRIOR_SUMMARY, render_summary(summary_text))


def _persist_trace(db, run, snapshot, history, estimated, total, tool_cost, summarized):
    model_id = snapshot.get("model_id", "")
    trace = {
        "model": model_id,
        "trigger": run.data.get("trigger_type", "interactive"),
        "context_version": 1,
        "context_window": (snapshot.get("context_window_info") or {}).get("context_window"),
        "input_budget": total,
        "estimated_input_tokens": estimated,
        "tool_schema_tokens": tool_cost,
        "history_messages": len(history or []),
        "summarized": summarized,
        "missing_image_refs": run.data.get("missing_image_refs", []),
        "task_state_present": bool(run.data.get("task_state")),
    }
    update(run, context_trace=trace)


def _reevaluate_auto_model(db, run, snapshot, available):
    """Reconsider Auto only between provider calls; keep the Run's saved policy."""
    routing = run.data.get("auto_routing") or {}
    if run.data.get("requested_model_id") != "auto" or not routing.get("dynamic_switching", snapshot.get("auto_dynamic_switching", True)):
        return snapshot
    history = run.data.get("history") or []
    content = routing.get("content", "")
    task_state = run.data.get("task_state") or {}
    if isinstance(task_state, dict):
        content += "\n" + str(task_state.get("plan", ""))[:2000]
        content += "\n" + str(task_state.get("pending", ""))[:1000]
    # Tool output informs this decision locally, without entering selection events.
    recent = history[-8:]
    for item in reversed(recent):
        if item.get("role") == "tool":
            content += "\n" + str(item.get("content", ""))[:4000]
    context_parts = [str(item.get("content", "")) for item in history]
    tools_required = bool(available)
    model, selection = select_auto_model(
        db, run.owner_id, routing["allowed_ids"], content, snapshot["mode"],
        routing.get("artifact_mimes", []), tools_required, context_parts,
        routing.get("reserved_output_tokens", 4096), run.request_key,
        routing.get("attachment_bytes", 0), current_model_id=snapshot["model_record_id"],
        priority=routing.get("priority", snapshot.get("auto_priority", "balanced")),
    )
    if not model:
        return snapshot
    provider = db.get(Provider, model.data.get("provider_id"))
    if not provider or provider.owner_id != run.owner_id:
        return snapshot
    try:
        reasoning = resolve_reasoning(
            provider.data["kind"], model.data["model_id"], effective_capabilities(model.data),
            snapshot["mode"], snapshot.get("model_settings", {}),
        )
    except ValueError:
        return snapshot
    previous = snapshot["model_record_id"]
    selection["model_record_id"] = model.id
    selection["model_id"] = model.data["model_id"]
    selection["attempts"] = run.data.get("auto_selection", {}).get("attempts", [])
    if model.id != previous:
        snapshot = {
            **snapshot, "model_id": model.data["model_id"], "model_record_id": model.id,
            "provider": provider.data, "provider_record_id": provider.id,
            "reasoning": reasoning, "context_window_info": context_budget.resolve_window(model.data, snapshot),
        }
        changes = [*run.data.get("auto_model_history", []), {
            "step": run.data.get("steps", 0) + 1,
            "from_model_record_id": previous, "to_model_record_id": model.id,
            "model_id": model.data["model_id"],
        }]
        update(run, snapshot=snapshot, auto_model_history=changes)
        emit(db, run.id, "model_rerouted", {
            "from_model_record_id": previous, "to_model_record_id": model.id,
            "selection": selection,
        })
    update(run, auto_selection=selection)
    db.commit()
    return snapshot


async def drive(run_id):
    try:
        with SessionLocal() as db:
            run = db.get(Run, run_id)
            if not run or run.status in TERMINAL:
                return
            if run.status == "waiting_approval" and run.data.get("browser_manual_active"):
                return
            snapshot = run.data["snapshot"]
            # The DB status decides whether a Run can start: queued and
            # waiting_approval reopen are the only legal entry points.  An
            # already-running (stale) or unexpected status is never overwritten.
            if not can_transition(run.status, "running"):
                return
            transition_run(db, run, "running")
            if run.data.get("auto_selection"):
                emit(db, run.id, "model_selected", run.data["auto_selection"])
            db.commit()
            while True:
                db.refresh(run)
                if run.status == "cancelled":
                    return
                if run.data.get("browser_pause_requested"):
                    update(run, browser_pause_requested=False, browser_manual_active=True, browser_paused_at=now().isoformat())
                    transition_run(db, run, "paused")
                    db.commit()
                    return
                elapsed = (now() - run.created_at.replace(tzinfo=UTC)).total_seconds()
                if elapsed >= snapshot.get("max_seconds", 900):
                    if snapshot.get("policy", {}).get("checkpointing"):
                        finish(db, run, "interrupted", agent_interruption_reason(run, "実行時間の上限に到達しました。途中成果を確認して、必要なら予算を調整して再開してください。"))
                    else:
                        finish(db, run, "completed", "このモードの実行時間上限に到達したため、取得済みの結果で終了しました。長い作業には長作業モードを使用してください。")
                    return
                calls = list(
                    db.scalars(
                        select(ToolCall)
                        .where(
                            ToolCall.run_id == run.id, ToolCall.status.in_(["pending", "waiting_approval"])
                        )
                        .order_by(ToolCall.created_at)
                    )
                )
                prepared = []
                waiting_for_approval = False
                attempt_counts = completed_tool_attempts(db, run.id)
                for call in calls:
                    current = registry(db, run.owner_id).get(call.data["tool_id"])
                    result = None
                    if not current or fingerprint(current) != call.data["tool_version"]:
                        result = {"error": "Tool definition changed", "code": "tool_definition_changed", "type": "tool_definition_changed"}
                    else:
                        key = tool_attempt_key(current["id"], call.data["arguments"])
                        if attempt_counts[key] >= SAME_TOOL_ARGUMENT_LIMIT:
                            result = {"error": "Tool loop detected", "code": "tool_loop_detected", "type": "tool_loop_detected"}
                        else:
                            attempt_counts[key] += 1
                            try:
                                decision = permission(db, run, current, call.data["arguments"])
                            except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
                                decision = "deny"
                            if decision == "deny":
                                result = {"error": "Tool denied", "code": "tool_denied", "type": "tool_denied"}
                            elif decision == "ask" and run.data.get("scheduled_run_id"):
                                result = {"error": "Scheduled runs require an always-allowed tool", "code": "tool_denied", "type": "tool_denied"}
                            elif decision == "ask":
                                approval = db.scalar(select(Approval).where(Approval.tool_call_id == call.id))
                                if not approval:
                                    approval = Approval(
                                        owner_id=run.owner_id,
                                        run_id=run.id,
                                        tool_call_id=call.id,
                                        expires=now() + timedelta(hours=24),
                                        data={
                                            "tool": current["model_name"],
                                            "arguments": call.data["arguments"],
                                            "tool_version": fingerprint(current),
                                            "scope": call_scope(current, call.data["arguments"]),
                                            "risk": current["risk"],
                                        },
                                    )
                                    db.add(approval)
                                    db.flush()
                                    emit(db, run.id, "approval", {"id": approval.id, **approval.data})
                                    from mix_agent.wakeups import mark
                                    mark(db, "scheduler")
                                if (
                                    approval.expires.replace(tzinfo=UTC) < now()
                                    and approval.status == "pending"
                                ):
                                    approval.status = "expired"
                                if approval.status == "pending":
                                    call.status = "waiting_approval"
                                    waiting_for_approval = True
                                if approval.status not in ("once", "always"):
                                    result = {"error": "Tool denied", "code": "tool_denied", "type": "tool_denied"}
                    prepared.append((call, current, result))
                if waiting_for_approval:
                    transition_run(db, run, "waiting_approval")
                    db.commit()
                    return
                index = 0
                while index < len(prepared):
                    call, current, result = prepared[index]
                    if result is not None:
                        complete_tool_call(db, run, call, current, result)
                        index += 1
                        continue
                    # Only a consecutive sequence of explicitly-safe tools may overlap.
                    batch = [(call, current)]
                    index += 1
                    if is_parallel_safe(current):
                        while index < len(prepared) and len(batch) < PARALLEL_TOOL_LIMIT:
                            next_call, next_current, next_result = prepared[index]
                            if next_result is not None or not is_parallel_safe(next_current):
                                break
                            batch.append((next_call, next_current))
                            index += 1
                    await execute_prepared_calls(db, run, snapshot, batch)
                db.refresh(run)
                if run.status == "cancelled":
                    return
                if run.data.get("browser_pause_requested"):
                    update(run, browser_pause_requested=False, browser_manual_active=True, browser_paused_at=now().isoformat())
                    transition_run(db, run, "paused")
                    db.commit()
                    return
                # Periodic durable checkpoint so resume can pick a recent step.
                from mix_agent.runs import checkpoints as run_checkpoints
                if (
                    snapshot.get("policy", {}).get("checkpointing")
                    and run_checkpoints.should_checkpoint(snapshot, int(run.data.get("steps", 0)))
                ):
                    try:
                        checkpoint = run_checkpoints.save(db, run, trigger="interval")
                        emit(db, run.id, "checkpoint_saved", {
                            "id": checkpoint.id,
                            "step": checkpoint.step,
                            "trigger": checkpoint.trigger,
                            "tool_count": checkpoint.tool_count,
                        })
                        db.commit()
                    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
                        # A failed checkpoint must never break the drive loop.
                        db.rollback()
                # Stagnation self-repair: detect repeated failures / no-progress
                # patterns and inject a corrective user-role entry on the fly.
                if snapshot.get("policy", {}).get("stagnation_detection"):
                    try:
                        from mix_agent.runs import stagnation as run_stagnation
                        findings = run_stagnation.detect_stagnation(db, run)
                        if findings:
                            already = run.data.get("last_stagnation_step")
                            current_step = int(run.data.get("steps", 0))
                            if already is None or current_step - int(already) >= 3:
                                msg = run_stagnation.repair_history_message(findings)
                                if msg:
                                    update(
                                        run,
                                        history=[*run.data["history"], {"role": "user", "content": msg}],
                                        last_stagnation_step=current_step,
                                        stagnation_findings=[
                                            {"code": f["code"], "severity": f.get("severity"),
                                             "tool_id": f.get("tool_id"), "count": f.get("count")}
                                            for f in findings
                                        ],
                                    )
                                    emit(db, run.id, "stagnation_detected", {
                                        "findings": [
                                            {"code": f["code"], "severity": f.get("severity"),
                                             "tool_id": f.get("tool_id"), "count": f.get("count")}
                                            for f in findings
                                        ],
                                        "step": current_step,
                                    })
                                    db.commit()
                    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
                        db.rollback()
                mode = snapshot["mode"]
                steps = int(run.data.get("steps", 0))
                count = int(run.data.get("tool_count", 0))
                call_limit = snapshot.get("max_tool_calls", 50 if mode == "agent" else 8)
                step_limit = snapshot.get("max_steps", 8)
                seconds_limit = snapshot.get("max_seconds", 900)
                available = [t for t in snapshot["tools"] if t["id"] in registry(db, run.owner_id)]
                tool_limit_reached = count >= call_limit
                if tool_limit_reached:
                    available = []
                step_limit_reached = steps >= step_limit
                # Budget extension request: when policy allows it and limits are
                # close to exhaustion, pause and ask the user before stopping.
                if (
                    snapshot.get("policy", {}).get("budget_extension")
                    and (tool_limit_reached or step_limit_reached)
                ):
                    extensions_used = int(run.data.get("budget_extensions_used", 0))
                    extensions_max = int(snapshot.get("policy", {}).get("max_budget_extensions", 0))
                    if extensions_used < extensions_max:
                        update(
                            run,
                            budget_extension_request={
                                "tool_calls_used": count,
                                "tool_calls_limit": call_limit,
                                "steps_used": steps,
                                "steps_limit": step_limit,
                                "elapsed_seconds": int(elapsed),
                                "max_seconds": int(seconds_limit),
                                "requested_at": now().isoformat(),
                            },
                            budget_extensions_used=extensions_used + 1,
                        )
                        emit(db, run.id, "budget_extension_requested", {
                            "tool_calls_used": count, "tool_calls_limit": call_limit,
                            "steps_used": steps, "steps_limit": step_limit,
                            "elapsed_seconds": int(elapsed), "max_seconds": int(seconds_limit),
                            "extensions_used": extensions_used + 1,
                            "extensions_max": extensions_max,
                        })
                        transition_run(db, run, "budget_extension_pending")
                        db.commit()
                        return
                if tool_limit_reached and snapshot.get("policy", {}).get("checkpointing"):
                    finish(db, run, "interrupted", agent_interruption_reason(run, "Tool Call数の上限に到達しました。途中成果を確認して再開してください。"))
                    return
                if tool_limit_reached:
                    available = []
                if step_limit_reached and snapshot.get("policy", {}).get("checkpointing"):
                    finish(db, run, "interrupted", agent_interruption_reason(run, "モデル呼び出し回数の上限に到達しました。途中成果を確認して再開してください。"))
                    return
                if step_limit_reached:
                    available = []
                snapshot = _reevaluate_auto_model(db, run, snapshot, available)
                provider = snapshot["provider"]
                # Credentials are resolved at use time, never copied into run snapshots.
                key = read_secret(db, provider.get("secret_id"))
                history = run.data["history"]
                # Model-aware pre-send budget check with progressive compaction.
                try:
                    send_history = await _maybe_compact_context(db, run, snapshot, provider, key)
                except ContextBudgetError as exc:
                    finish(db, run, "interrupted", str(exc))
                    return
                history = run.data["history"]
                emit(db, run.id, "model_started", {
                    "step": steps + 1,
                    "mode": mode,
                    "model_id": snapshot.get("model_id"),
                    "model_record_id": snapshot.get("model_record_id"),
                    "provider_id": snapshot.get("provider_record_id"),
                })
                update(run, steps=steps + 1)
                db.commit()
                response = None
                provider_started_at = now()
                first_output_at = None
                buffered_text = []
                last_text_flush_at = asyncio.get_running_loop().time()

                model_settings = {
                    k: v for k, v in snapshot.get("model_settings", {}).items() if k != "_resolved_reasoning"
                }
                if "reasoning" in snapshot:
                    model_settings["_resolved_reasoning"] = snapshot["reasoning"]
                # One stable session id per conversation for gateway routing/caching.
                model_settings["_session_id"] = run.conversation_id
                try:
                    async with asyncio.timeout(max(1, snapshot.get("max_seconds", 900) - elapsed)):
                        await wait_for_provider_slot({**provider, "id": snapshot.get("provider_record_id")})
                        async for event in Adapter(provider, key).stream(
                            snapshot["model_id"], (
                                [*send_history, {"role": "user", "content": "The step or tool-call limit for this response mode has been reached. Do not request more tools; answer the original request using the available information and recommend Long work mode if sustained work remains."}]
                                if tool_limit_reached or step_limit_reached else send_history
                            ), available, mode, model_settings
                        ):
                            if event["kind"] == "response":
                                if flush_stream_text(db, run.id, buffered_text):
                                    last_text_flush_at = asyncio.get_running_loop().time()
                                response = event
                            elif event["kind"] == "text":
                                if event.get("text") and first_output_at is None:
                                    first_output_at = now()
                                buffered_text.append(event.get("text", ""))
                                if asyncio.get_running_loop().time() - last_text_flush_at >= STREAM_TEXT_FLUSH_SECONDS:
                                    flush_stream_text(db, run.id, buffered_text)
                                    last_text_flush_at = asyncio.get_running_loop().time()
                            else:
                                # Do not let a reasoning/activity event overtake
                                # already received text in the durable stream.
                                if flush_stream_text(db, run.id, buffered_text):
                                    last_text_flush_at = asyncio.get_running_loop().time()
                                emit(db, run.id, event["kind"], {"text": event.get("text", "")})
                                db.commit()
                        flush_stream_text(db, run.id, buffered_text)
                        if response is None:
                            raise ProviderIncompleteResponseError("Provider stream ended without a final response")
                except Exception as exc:
                    # A failed stream must still expose every fragment the
                    # provider already produced before reporting its outcome.
                    flush_stream_text(db, run.id, buffered_text)
                    retryable = (
                        is_retryable_provider_error(exc)
                        or is_nvidia_nim_function_not_found(provider, exc)
                        or is_nvidia_nim_chat_incompatible(provider, snapshot["model_id"], exc)
                    )
                    classification = classify_failure(exc)
                    retry_until = retry_after(exc)
                    if run.data.get("requested_model_id") == "auto" and snapshot.get("provider_record_id"):
                        record_reliability(
                            db, run.owner_id, snapshot["model_record_id"], snapshot["provider_record_id"],
                            usage_scope(mode, bool(run.data.get("auto_routing", {}).get("tools_required"))),
                            "failure", classification, retry_until,
                            profile=run.data.get("auto_selection", {}).get("profile"),
                            required_tokens=run.data.get("auto_selection", {}).get("required_tokens"),
                        )
                    rerouted = (
                        await reroute_after_provider_error(db, run, run.request_key, exc, classification, retry_until)
                        if run.data.get("requested_model_id") == "auto" and retryable and first_output_at is None
                        else None
                    )
                    if rerouted:
                        snapshot = rerouted
                        continue
                    # Pinned (non-Auto) models had no retry path: the first
                    # transient 503/429/timeout killed a 90-minute Run. Allow one
                    # bounded retry on genuinely transient provider errors. Never
                    # replay a partially streamed answer — once text is visible,
                    # repeating the request would create a duplicate response.
                    # NVIDIA NIM "function not found" and chat-incompatible 400s
                    # are NOT retried: they are deterministic per-model failures,
                    # and the Auto path already routes around them.
                    retries = int(run.data.get("provider_retry_attempts", 0))
                    is_pinned = run.data.get("requested_model_id") != "auto"
                    transient = is_pinned and is_retryable_provider_error(exc) and first_output_at is None
                    retry_budget = max(0, seconds_limit - elapsed)
                    if transient and retries < 1 and retry_budget > 0:
                        delay = max(0.0, (retry_until - now()).total_seconds()) if retry_until else 0.5
                        if delay < retry_budget:
                            update(
                                run,
                                provider_retry_attempts=retries + 1,
                                steps=max(0, int(run.data.get("steps", 0)) - 1),
                                last_provider_retry={
                                    "classification": classification,
                                    "error_type": type(exc).__name__,
                                    "attempt": retries + 1,
                                    "at": now().isoformat(),
                                },
                            )
                            db.commit()
                            if delay:
                                await asyncio.sleep(delay)
                            continue
                    raise
                model_record = db.get(Model, snapshot["model_record_id"]) if snapshot.get("model_record_id") else None
                record_usage(
                    db, run.owner_id, model_record.data if model_record else {},
                    snapshot.get("provider_record_id"), mode, response.get("usage", {}),
                    run_id=run.id,
                    input_estimate=(run.data.get("context_trace") or {}).get("estimated_input_tokens"),
                    model_record_id=snapshot.get("model_record_id"),
                )
                if run.data.get("requested_model_id") == "auto" and snapshot.get("provider_record_id"):
                    completed_at = now()
                    first_output_at = first_output_at or completed_at
                    record_reliability(
                        db, run.owner_id, snapshot["model_record_id"], snapshot["provider_record_id"],
                        usage_scope(mode, bool(run.data.get("auto_routing", {}).get("tools_required"))), "success",
                        first_output_ms=max(1, round((first_output_at - provider_started_at).total_seconds() * 1000)),
                        completion_ms=max(1, round((completed_at - provider_started_at).total_seconds() * 1000)),
                        output_tokens=output_tokens(response.get("usage", {})),
                        profile=run.data.get("auto_selection", {}).get("profile"),
                        required_tokens=run.data.get("auto_selection", {}).get("required_tokens"),
                    )
                message = response["message"]
                update(run, history=[*history, message])
                tool_calls = response.get("tool_calls", [])
                if tool_calls:
                    by_name = {t["model_name"]: t for t in available}
                    remaining_calls = max(0, call_limit - count)
                    for index, c in enumerate(tool_calls):
                        tool = by_name.get(c["name"])
                        if not tool:
                            # A hallucinated or renamed Tool is a recoverable
                            # model mistake: report it back so the next turn can
                            # correct itself. Raising here used to fail a Run
                            # that may already have spent an hour of budget.
                            _reject_tool_call(
                                db, run, c, available,
                                error="The requested tool is not available in this Run.",
                                code="tool_unavailable",
                            )
                            continue
                        try:
                            args = json.loads(c["arguments"]) if isinstance(c["arguments"], str) else c["arguments"]
                        except (TypeError, ValueError):
                            _reject_tool_call(
                                db, run, c, available,
                                error="The tool arguments were not valid JSON. Send a JSON object matching the tool schema.",
                                code="tool_arguments_invalid_json",
                            )
                            continue
                        if not isinstance(args, dict):
                            _reject_tool_call(
                                db, run, c, available,
                                error="The tool arguments must be a JSON object.",
                                code="tool_arguments_not_object",
                            )
                            continue
                        call = ToolCall(
                            owner_id=run.owner_id,
                            run_id=run.id,
                            data={
                                "tool_id": tool["id"],
                                "tool_version": fingerprint(tool),
                                "provider_call_id": c["id"],
                                "name": c["name"],
                                "arguments": args,
                                # Keep presentation metadata stable even when a Tool is edited later.
                                "activity": activity_summary(tool["id"], args),
                                "risk": tool.get("risk", "external"),
                            },
                        )
                        db.add(call)
                        db.flush()
                        if index >= remaining_calls:
                            complete_tool_call(
                                db, run, call, tool,
                                {"error": "Tool call limit reached", "code": "tool_call_limit_reached", "type": "tool_call_limit_reached"},
                            )
                    update(run, tool_count=min(call_limit, count + len(tool_calls)))
                    if snapshot.get("policy", {}).get("checkpointing"):
                        update(run, checkpoint={"status": "working", "steps": steps + 1, "tool_count": min(call_limit, count + len(tool_calls))})
                        # Persist a durable checkpoint at every step boundary so
                        # the user can pick the most recent working state.
                        try:
                            from mix_agent.runs import checkpoints as run_checkpoints
                            checkpoint = run_checkpoints.save(db, run, trigger="working")
                            emit(db, run.id, "checkpoint_saved", {
                                "id": checkpoint.id,
                                "step": checkpoint.step,
                                "trigger": checkpoint.trigger,
                                "tool_count": checkpoint.tool_count,
                            })
                        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
                            db.rollback()
                    db.commit()
                    continue
                if not is_user_facing_answer(message["content"]):
                    repairs = run.data.get("answer_repair_attempts", 0)
                    if repairs < 1:
                        update(
                            run,
                            answer_repair_attempts=repairs + 1,
                            history=[
                                *run.data["history"],
                                {
                                    "role": "user",
                                    "content": "The previous turn did not provide a user-facing answer. A progress announcement such as 'I will check' does not complete the user's request and is not a final answer. Continue now: perform the announced action with an available tool when appropriate, answer the original request using the available results, or ask a genuinely necessary clarifying question. Do not return another progress promise, raw JSON, a URL alone, or an empty response.",
                                },
                            ],
                        )
                        db.commit()
                        continue
                    message = {"role": "assistant", "content": answer_fallback(run.data["history"])}
                    update(run, history=[*history, message])
                    update(run, answer_evaluation={"status": "needs_review", "reason": "自動補足で終了しました。依頼の達成を確認してください。"})
                else:
                    update(run, answer_evaluation={"status": "provided", "reason": "ユーザー向けの最終回答を確認しました。内容の正しさは自動判定していません。"})
                issues = agent_completion_issues(run, list(db.scalars(select(ToolCall).where(ToolCall.run_id == run.id).order_by(ToolCall.created_at))))
                if issues:
                    repairs = run.data.get("agent_completion_repairs", 0)
                    if run.data.get("agent_plan") and repairs < 1 and steps < snapshot.get("max_steps", 8):
                        update(run, agent_completion_repairs=repairs + 1,
                               history=[*run.data["history"], {"role": "user", "content":
                               "The task is not yet verified:\n- " + "\n- ".join(issues) + "\nContinue the work, update the plan with remaining tasks and verification, then answer. If blocked, report the blocker and remaining work."}])
                        db.commit()
                        continue
                    # Keep the model's useful partial answer visible even when
                    # the Agent cannot satisfy the verification gate. The Run
                    # remains interrupted and the answer is explicitly marked
                    # for review; silently discarding it made a successful
                    # final response disappear because of a bookkeeping gate.
                    update(run, answer_evaluation={
                        "status": "needs_review",
                        "reason": "作業結果を自動確認できませんでした。回答と残作業を確認してください。",
                    })
                    db.add(Message(owner_id=run.owner_id, conversation_id=run.conversation_id, data={
                        "role": "assistant",
                        "content": message["content"],
                        "run_id": run.id,
                        "artifacts": [a for a in run.data.get("artifacts", []) if is_user_artifact(a)],
                        "answer_evaluation": run.data["answer_evaluation"],
                    }))
                    finish(db, run, "interrupted", agent_interruption_reason(run, issues[0]))
                    return
                performance = None
                output_count = output_tokens(response.get("usage", {}))
                # A provider's final usage is authoritative.  The first visible
                # text timestamp excludes request setup and initial waiting time.
                if output_count and first_output_at is not None:
                    completed_at = now()
                    generation_ms = max(1, round((completed_at - first_output_at).total_seconds() * 1000))
                    first_output_ms = max(1, round((first_output_at - provider_started_at).total_seconds() * 1000))
                    event = record_performance(
                        db, run.owner_id, snapshot["model_record_id"], snapshot["provider_record_id"],
                        mode, output_count, generation_ms, first_output_ms,
                    )
                    db.add(event)
                    performance = {
                        "output_tokens": output_count,
                        "generation_ms": generation_ms,
                        "first_output_ms": first_output_ms,
                        "tokens_per_second": event.data["tokens_per_second"],
                    }
                message_data = {
                    "role": "assistant",
                    "content": message["content"],
                    "run_id": run.id,
                    "artifacts": [a for a in run.data.get("artifacts", []) if is_user_artifact(a)],
                }
                if performance:
                    message_data["performance"] = performance
                if run.data.get("auto_selection"):
                    selected_model = db.get(Model, snapshot["model_record_id"])
                    message_data["auto_selection"] = {
                        **run.data["auto_selection"],
                        "model_record_id": snapshot["model_record_id"],
                        "model_id": snapshot["model_id"],
                        "model_name": (selected_model.data.get("name") or snapshot["model_id"]) if selected_model else snapshot["model_id"],
                    }
                db.add(Message(owner_id=run.owner_id, conversation_id=run.conversation_id, data=message_data))
                user_content = next((item.get("content", "") for item in reversed(run.data["history"]) if item.get("role") == "user"), "")
                if not run.data.get("temporary_mode"):
                    # Synchronous Memory Evaluator pass: harvest any Task Memory
                    # the agent built up during the Run (Goals / Decisions /
                    # Failures / Experiences) before the async trace job runs.
                    try:
                        from mix_agent.memory.runtime import apply_plan, evaluate

                        # Scope the optional synchronous memory pass to a
                        # SAVEPOINT. A full db.rollback() here used to discard
                        # the assistant Message and Run updates already staged
                        # for completion, turning a memory failure into a
                        # completed Run with no answer.
                        with db.begin_nested():
                            for observation in _collect_memory_observations(db, run, message["content"]):
                                plan = evaluate(
                                    db, run.owner_id,
                                    task_id=run.id,
                                    observation=observation,
                                    explicit_user=bool(observation.get("explicit_user")),
                                ).get("plan") or []
                                apply_plan(db, run.owner_id, plan, run_id=run.id)
                    except Exception:  # noqa: BLE001 - synchronous memory writes must never block completion
                        # begin_nested() rolls back only the memory SAVEPOINT.
                        pass
                    from mix_agent.memory.jobs import enqueue as enqueue_memory
                    enqueue_memory(db, run, user_content, message["content"], run.data.get("memory_trace_ids", []))
                conversation = db.get(Conversation, run.conversation_id)
                if conversation:
                    conversation.data = {**conversation.data, "last_message_at": now().isoformat()}
                emit(db, run.id, "message", {"content": message["content"], "usage": response.get("usage", {})})
                finish(db, run, "completed")
                return
    except asyncio.CancelledError:
        epoch = _current_launch_epoch()
        with SessionLocal() as db:
            run = db.get(Run, run_id)
            if run and run.status not in TERMINAL and run.data.get("launch_epoch") == epoch:
                finish(db, run, "cancelled", "ユーザーが停止しました")
        for kind in ("execution", "mcp"):
            try:
                await runner_request(kind, "/cancel", {"run_id": run_id}, timeout=5)
            except Exception:  # noqa: BLE001, S110 - intentionally classified; never leak raw details
                pass
    except Exception as exc:  # noqa: BLE001 - intentionally classified; never leak raw details
        epoch = _current_launch_epoch()
        with SessionLocal() as db:
            run = db.get(Run, run_id)
            if run and run.data.get("launch_epoch") == epoch:
                finish(
                    db,
                    run,
                    "failed",
                    provider_failure_reason(exc, run.data.get("snapshot", {}).get("provider", {})),
                )
    finally:
        TASKS.pop(run_id, None)
        try:
            with SessionLocal() as db:
                ended = db.get(Run, run_id)
                should_close = ended is None or ended.status in TERMINAL
            if should_close:
                await runner_request("browser", "/browser-close", {"run_id": run_id}, timeout=5)
        except Exception:
            pass


async def scheduler():
    interrupted_browser_runs = []
    with SessionLocal() as db:
        for run in db.scalars(select(Run).where(Run.status == "running")):
            interrupted_browser_runs.append(run.id)
            finish(db, run, "interrupted", "サーバーが再起動しました。結果不明の操作は自動再実行しません。")
        from mix_agent.schedules import reconcile
        reconcile(db, launch)
    for interrupted_id in interrupted_browser_runs:
        try:
            await runner_request("browser", "/browser-close", {"run_id": interrupted_id}, timeout=5)
        except Exception:
            pass
    from mix_agent.wakeups import scheduler as wakeup
    seen = wakeup.revision
    while True:
        delay = 30.0  # durable fallback if a process-local signal is missed
        try:
            from mix_agent.storage import backup

            if backup.ACTIVE:
                delay = 2.0
            else:
                with SessionLocal() as db:
                    from mix_agent.api.routes import purge_expired_conversations
                    purge_expired_conversations(db)
                    from mix_agent.schedules import tick
                    tick(db, launch)
                    # Confirm unknown/stale model capabilities in the background so
                    # Auto does not depend on a user-triggered check.  A hard
                    # timeout keeps a slow provider from stalling the tick.
                    try:
                        from mix_agent import config
                        from mix_agent.api.routes import probe_due_models
                        await asyncio.wait_for(
                            probe_due_models(db), timeout=config.PROBE_TICK_TIMEOUT_SECONDS
                        )
                    except Exception:
                        db.rollback()
                        import logging
                        logging.getLogger(__name__).exception("capability probe pass failed")
                    current = now()
                    # Cron is minute-granular, so wake exactly for its next boundary.
                    delay = max(0.05, 60 - current.second - current.microsecond / 1_000_000)
                    for run in db.scalars(select(Run).where(Run.status.in_(["queued", "waiting_approval"]))):
                        if run.status == "queued":
                            launch(run.id)
                            continue
                        approval = db.scalar(
                            select(Approval).where(Approval.run_id == run.id, Approval.status == "pending")
                        )
                        if not approval or approval.expires.replace(tzinfo=UTC) < current:
                            launch(run.id)
                        else:
                            delay = min(delay, max(0.05, (approval.expires.replace(tzinfo=UTC) - current).total_seconds()))
                    for scheduled in db.scalars(select(ScheduledRun).where(ScheduledRun.status == "retrying")):
                        try:
                            retry_at = datetime.fromisoformat(scheduled.data.get("retry_at", ""))
                            delay = min(delay, max(0.05, (retry_at - current).total_seconds()))
                        except (TypeError, ValueError):
                            pass
        except Exception:
            delay = 30.0
            import logging
            logging.getLogger(__name__).exception("scheduler tick failed")
        seen = await wakeup.wait(seen, timeout=delay)
