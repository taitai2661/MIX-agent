import asyncio
import json
import re
from collections import defaultdict, deque
from datetime import timedelta
from urllib.parse import urlsplit

from sqlalchemy import func, select

from mix_agent.context import head_blocks
from mix_agent.context import task_state as context_task_state
from mix_agent.db.models import (
    Event,
    ScheduledRun,
    ToolCall,
    now,
)
from mix_agent.runs.state import TERMINAL, transition_run


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
    """Decision (Phase 2): keep harvesting nothing synchronously; always [].

    The roadmap asked whether a run's decisions / artifact summaries should be
    collected here, on the completion path, in addition to the async extractor.
    Evaluated against ``docs/memory-spec.md`` and rejected for now:

    * §0 limits Memory to durable facts, preferences and constraints **about
      the user**. A run's decisions and artifact summaries are task output;
      storing them turns Memory into a transcript and dilutes retrieval for
      every later turn. Keeping task memory separate is exactly what §1's
      ``task_id`` and §3's lifecycle (``superseded`` / ``expired``) are for —
      and both are Phase 3 (``未実装``). Without an expiry path, harvesting
      now only accumulates decisions that can never go stale.
    * The async ``MemoryProcessingJob`` (``memory.jobs.enqueue``) already gets
      this same turn, and judges it with more context and more gates: the
      spec §6 prompt, the top-10 ``existing`` memories for dedupe, §8 secret
      filtering, 3-attempt backoff, and the ``memory_auto_formation`` setting.
      A second synchronous writer would repeat that judgment with none of the
      gates — and would run inside the run-completion transaction, adding
      latency and a failure surface to every answer.
    * Roadmap Phase 2 marks this as "実験的に有効化できるか検証" — an
      experiment, not a shipped behaviour. The safe place for it is behind
      Phase 3's consolidation, where a ``decision``-role observation can be
      stored with ``task_id=run.id`` and decayed, rather than as a free-form
      summary on the hot path.

    Kept as a stub so the call site in ``drive`` stays unchanged; returns [].
    """
    return []


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
    update(
        run,
        history=[
            *run.data["history"],
            {
                "role": "tool",
                "call_id": provider_call.get("id") or "",
                "name": name or "unknown",
                "content": content,
            },
        ],
    )
    emit(
        db,
        run.id,
        "tool_result",
        {
            "id": call.id,
            "name": call.data["name"],
            "result": {"error": error, "code": code, "type": code},
            "activity": call.data["activity"],
        },
    )
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
        error["message"] = messages.get(
            error["code"],
            "The tool execution failed. Choose an appropriate next action or answer from the information already available.",
        )
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
        "承知しました",
        "了解しました",
        "かしこまりました",
        "わかりました",
        "understood",
        "sure",
        "okay",
        "ok",
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
        state = (
            "失敗"
            if isinstance(result, dict) and (result.get("error") or result.get("status") == "failed")
            else "完了"
        )
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
        if all(
            call.data.get("risk") == "read"
            and not (call.data.get("result") or {}).get("error")
            and (call.data.get("result") or {}).get("ok", True)
            for call in completed
        ):
            return None
        return "作業計画と残作業が記録されていません。"
    if plan.get("pending"):
        return "未完了の作業があります: " + "、".join(plan["pending"][:5])
    if not plan.get("verification"):
        return "成果を確認した根拠が記録されていません。"
    changes = [
        index
        for index, call in enumerate(completed)
        if call.data.get("risk") != "read" and call.data.get("tool_id") != "update_plan"
    ]
    if changes:
        check_ids = {
            "read_file",
            "files_list",
            "search_files",
            "workspace_check",
            "browser_read",
            "browser_screenshot",
            "browser_extract",
            "web_search",
            "web_fetch",
            "web_fetch_pdf",
            "knowledge_search",
            "memory_search",
            "skill_search",
            "skill_resource",
            "schedule_list",
            "process_list",
        }
        if not any(
            call.data.get("tool_id") in check_ids
            and isinstance(call.data.get("result"), dict)
            and not call.data["result"].get("error")
            and call.data["result"].get("ok", True)
            for call in completed[changes[-1] + 1 :]
        ):
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
    if status in {"failed", "cancelled", "interrupted"} and run.data.get("answer_evaluation", {}).get(
        "status"
    ) not in {"provided", "needs_review"}:
        update(
            run, answer_evaluation={"status": "unanswered", "reason": "最終回答の前に実行が終了しました。"}
        )
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
                emit(
                    db,
                    run.id,
                    "checkpoint_saved",
                    {
                        "id": checkpoint.id,
                        "step": checkpoint.step,
                        "trigger": checkpoint.trigger,
                        "tool_count": checkpoint.tool_count,
                    },
                )
            except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
                db.rollback()
    scheduled_id = run.data.get("scheduled_run_id")
    if scheduled_id:
        scheduled = db.get(ScheduledRun, scheduled_id)
        if scheduled:
            retry = status == "failed" and int(scheduled.data.get("attempt", 0)) == 0
            scheduled.status = "retrying" if retry else ("completed" if status == "completed" else "failed")
            scheduled.data = {
                **scheduled.data,
                "reason": reason,
                "finished_at": now().isoformat(),
                **({"attempt": 1, "retry_at": (now() + timedelta(seconds=30)).isoformat()} if retry else {}),
            }
            if retry:
                from mix_agent.wakeups import mark

                mark(db, "scheduler")
            from mix_agent.schedules import notify

            if not retry:
                notify(
                    db,
                    run.owner_id,
                    "schedule.completed" if status == "completed" else "schedule.failed",
                    "定期実行: " + ("完了" if status == "completed" else "失敗"),
                    scheduled,
                )
    db.commit()
    if run.data.get("temporary_mode"):
        from mix_agent.storage.cleanup import purge_temporary_run

        purge_temporary_run(db, run)
        db.commit()
