import asyncio
import json
import re
from collections import defaultdict, deque
from datetime import UTC, timedelta
from urllib.parse import urlsplit

from sqlalchemy import func, select

from mix_agent.auth.security import read_secret
from mix_agent.context import budget as context_budget
from mix_agent.context import task_state as context_task_state
from mix_agent.context import tokens as context_tokens
from mix_agent.context import tools_selector as context_tools
from mix_agent.context.builder import select_recent
from mix_agent.context.references import (
    DEFAULT_TOOL_INLINE_LIMIT,
    extract_data_url_images,
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
    is_nvidia_nim_chat_incompatible,
    is_nvidia_nim_function_not_found,
    is_retryable_provider_error,
)
from mix_agent.providers.reasoning import resolve_reasoning
from mix_agent.reliability import classify_failure, retry_after, usage_scope
from mix_agent.reliability import record as record_reliability
from mix_agent.routing import effective_capabilities, select_auto_model
from mix_agent.tools.execute import execute, runner_request
from mix_agent.tools.registry import call_scope, fingerprint, permission, registry

TASKS = {}
TERMINAL = {"completed", "failed", "cancelled", "interrupted"}
PARALLEL_TOOL_LIMIT = 4
SAME_TOOL_ARGUMENT_LIMIT = 3
_PROVIDER_RATE_LOCK = asyncio.Lock()
_PROVIDER_REQUESTS = defaultdict(deque)


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


def emit(db, run_id, kind, data):
    sequence = (db.scalar(select(func.max(Event.sequence)).where(Event.run_id == run_id)) or 0) + 1
    db.add(Event(run_id=run_id, sequence=sequence, kind=kind, data=data))
    db.flush()


def update(run, **fields):
    run.data = {**run.data, **fields}


def finish(db, run, status, reason=None):
    run.status = status
    fields = {"reason": reason}
    if run.data.get("snapshot", {}).get("policy", {}).get("checkpointing"):
        fields["checkpoint"] = {
            "status": status,
            "steps": run.data.get("steps", 0),
            "tool_count": run.data.get("tool_count", 0),
            "finished_at": now().isoformat(),
        }
    update(run, **fields)
    emit(db, run.id, "status", {"status": status, "reason": reason})
    scheduled_id = run.data.get("scheduled_run_id")
    if scheduled_id:
        scheduled = db.get(ScheduledRun, scheduled_id)
        if scheduled:
            retry = status == "failed" and int(scheduled.data.get("attempt", 0)) == 0
            scheduled.status = "retrying" if retry else ("completed" if status == "completed" else "failed")
            scheduled.data = {**scheduled.data, "reason": reason, "finished_at": now().isoformat(),
                              **({"attempt": 1, "retry_at": (now() + timedelta(seconds=30)).isoformat()} if retry else {})}
            from mix_agent.schedules import notify
            if not retry:
                notify(db, run.owner_id, "schedule.completed" if status == "completed" else "schedule.failed", "定期実行: " + ("完了" if status == "completed" else "失敗"), scheduled)
    db.commit()
    if run.data.get("temporary_mode"):
        from mix_agent.api.routes import purge_temporary_run
        purge_temporary_run(db, run)
        db.commit()


def launch(run_id):
    if run_id not in TASKS or TASKS[run_id].done():
        TASKS[run_id] = asyncio.create_task(drive(run_id))


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
        prefer_other_provider_than=(provider_id if classification in {"rate_limit", "provider_5xx", "timeout"} else None),
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
    elif classification in {"provider_5xx", "timeout"} and provider.id == provider_id:
        await asyncio.sleep(min(4, 2 ** max(0, retry_number - 1)))
    return updated_snapshot


def provider_failure_reason(exc, provider=None):
    """Return a safe, actionable provider failure message without remote details."""
    response = getattr(exc, "response", None)
    status_code = getattr(response, "status_code", None) or getattr(exc, "status_code", None)
    error_name = type(exc).__name__
    if is_nvidia_nim_function_not_found(provider or {}, exc):
        return "NVIDIA NIMでこのアカウントに利用可能な推論機能が見つかりません。NVIDIA側の提供状況またはAPI Keyの利用権限を確認してください。"
    if status_code == 404 or error_name == "NotFoundError":
        return "選択したモデルがProviderに見つかりません。モデル一覧を更新するか、Providerの接続先とモデルIDを確認してください。"
    if status_code in (401, 403) or error_name in {"AuthenticationError", "PermissionDeniedError"}:
        return "Providerの認証または権限が拒否されました。API Keyと利用権限を確認してください。"
    if error_name == "ProviderConfigurationError":
        return "Providerの接続設定が不足しています。Provider設定を確認してください。"
    return "実行に失敗しました（" + error_name + "）。接続設定とモデル対応機能を確認してください。"


def complete_tool_call(db, run, call, current, result):
    """Persist one result in provider-call order after execution has settled."""
    call.status = "completed"
    summary = activity_result((current or {}).get("id", call.data["tool_id"]), result)
    call.data = {**call.data, "result": result, **({"result_activity": summary} if summary else {})}
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
    except Exception:
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
        except Exception:
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
    if isinstance(artifact, dict) and artifact.get("artifact_id"):
        existing = run.data.get("artifacts", [])
        if not any(item.get("artifact_id") == artifact["artifact_id"] for item in existing):
            update(run, artifacts=[*existing, artifact])
    if isinstance(artifact_info, dict) and artifact_info.get("artifact_id"):
        refs = list(run.data.get("tool_refs") or [])
        if artifact_info["artifact_id"] not in refs:
            update(run, tool_refs=[*refs, artifact_info["artifact_id"]])
        existing = run.data.get("artifacts", [])
        if not any(item.get("artifact_id") == artifact_info["artifact_id"] for item in existing):
            update(run, artifacts=[*existing, artifact_info])
    event_data = {"id": call.id, "name": call.data["name"], "result": result}
    if summary:
        event_data["activity"] = summary
    emit(db, run.id, "plan" if call.data["name"] == "update_plan" else "tool_result", event_data)
    db.commit()


async def execute_prepared_calls(db, run, snapshot, prepared):
    """Run a consecutive, already-authorized batch and preserve its result order."""
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
            return await asyncio.wait_for(
                execute(db, run, current, call.data["arguments"]), timeout=min(120, remaining)
            )
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            # Do not leak SDK headers, secrets, or remote response bodies.
            return {
                "error": "Tool failed",
                "code": "tool_timeout" if isinstance(exc, TimeoutError) else "tool_failed",
                "type": type(exc).__name__,
            }

    results = await asyncio.gather(*(one(call, current) for call, current in prepared))
    for (call, current), result in zip(prepared, results):
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
    except Exception:
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

    history = run.data.get("history") or []
    # Checkpoint: keep structured task state alive even for legacy runs.
    if not run.data.get("task_state"):
        first_user = next((m.get("content", "") for m in history if m.get("role") == "user"), "")
        update(run, task_state=context_task_state.ensure(None, first_user[:1000]))
    if run.data.get("summary") is None:
        update(run, summary={"text": "", "covered_count": 0, "updated_at": None})
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
                    snapshot["model_id"], summary_input, [], "chat", {"max_output_tokens": 2048}
                ):
                    if event["kind"] == "response":
                        summary_text = event["message"]["content"]
            summary_text = finalize_summary(summary_text)
            if summary_text:
                from mix_agent.db.models import now as _now

                update(run, summary={
                    "text": summary_text,
                    "covered_count": int((run.data.get("summary") or {}).get("covered_count", 0)) + len(evicted),
                    "updated_at": _now().isoformat(),
                })
                summarized = [{"evicted": len(evicted)}]
                emit(db, run.id, "context_summary", {"status": "completed", "summary": summary_text})
                # Rewrite head summary line progressively (no full re-summarization).
                head_text = (head[0].get("content") if head else "")
                head_text = _replace_summary_block(head_text, summary_text)
                if head:
                    head = [{**head[0], "content": head_text}]
                db.commit()
        except Exception:
            # Summarizer failure must not destroy state; keep old summary.
            emit(db, run.id, "context_summary", {"status": "failed"})
            db.commit()
    compacted = [*head, *recent]
    estimated = context_tokens.count_messages(compacted, model_id)
    if estimated > total + 2000 and len(compacted) <= 2:
        raise ContextBudgetError(
            f"context budget exceeded: estimated {estimated} > {total} tokens "
            f"(window {window_info.get('context_window')})"
        )
    update(run, history=compacted)
    _persist_trace(db, run, snapshot, compacted, estimated, total, tool_cost, summarized)
    db.commit()
    return _resolve_history_for_provider(db, run, compacted)


def _replace_summary_block(head_text: str, summary_text: str) -> str:
    rendered = render_summary(summary_text)
    marker = "Prior conversation summary (data):"
    if marker in head_text:
        pre, _, _post = head_text.partition(marker)
        # Replace only the old summary line (up to next block or end).
        lines = _post.split("\n")
        rest_index = next((i for i, line in enumerate(lines[1:], 1) if line and not line.startswith((" ", "\t")) and ":" in line[:40]), len(lines))
        _ = rest_index
        return pre + rendered
    return (head_text + "\n" + rendered) if head_text else rendered


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


async def drive(run_id):
    try:
        with SessionLocal() as db:
            run = db.get(Run, run_id)
            if not run or run.status in TERMINAL:
                return
            snapshot = run.data["snapshot"]
            run.status = "running"
            if run.data.get("auto_selection"):
                emit(db, run.id, "model_selected", run.data["auto_selection"])
            db.commit()
            while True:
                db.refresh(run)
                if run.status == "cancelled":
                    return
                elapsed = (now() - run.created_at.replace(tzinfo=UTC)).total_seconds()
                if elapsed >= snapshot.get("max_seconds", 900):
                    if snapshot.get("policy", {}).get("checkpointing"):
                        finish(db, run, "interrupted", "実行時間の上限に到達しました。途中成果を確認して、必要なら予算を調整して再開してください。")
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
                            except Exception:
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
                    run.status = "waiting_approval"
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
                mode = snapshot["mode"]
                steps = run.data.get("steps", 0)
                count = run.data.get("tool_count", 0)
                call_limit = snapshot.get("max_tool_calls", 50 if mode == "agent" else 8)
                provider = snapshot["provider"]
                # Credentials are resolved at use time, never copied into run snapshots.
                key = read_secret(db, provider.get("secret_id"))
                available = [t for t in snapshot["tools"] if t["id"] in registry(db, run.owner_id)]
                tool_limit_reached = count >= call_limit
                if tool_limit_reached:
                    available = []
                step_limit_reached = steps >= snapshot.get("max_steps", 8)
                if step_limit_reached and snapshot.get("policy", {}).get("checkpointing"):
                    finish(db, run, "interrupted", "モデル呼び出し回数の上限に到達しました。途中成果を確認して再開してください。")
                    return
                if step_limit_reached:
                    available = []
                history = run.data["history"]
                # Model-aware pre-send budget check with progressive compaction.
                try:
                    send_history = await _maybe_compact_context(db, run, snapshot, provider, key)
                except ContextBudgetError as exc:
                    finish(db, run, "interrupted", str(exc))
                    return
                history = run.data["history"]
                emit(db, run.id, "model_started", {"step": steps + 1, "mode": mode})
                update(run, steps=steps + 1)
                db.commit()
                response = None
                provider_started_at = now()
                first_output_at = None
                model_settings = {
                    k: v for k, v in snapshot.get("model_settings", {}).items() if k != "_resolved_reasoning"
                }
                if "reasoning" in snapshot:
                    model_settings["_resolved_reasoning"] = snapshot["reasoning"]
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
                                response = event
                            else:
                                if event["kind"] == "text" and first_output_at is None:
                                    first_output_at = now()
                                emit(db, run.id, event["kind"], {"text": event.get("text", "")})
                                db.commit()
                except Exception as exc:
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
                    if not rerouted:
                        raise
                    snapshot = rerouted
                    continue
                if response is None:
                    raise RuntimeError("Incomplete model response")
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
                            raise ValueError("Model requested an unavailable tool")
                        args = json.loads(c["arguments"]) if isinstance(c["arguments"], str) else c["arguments"]
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
                performance = None
                output_count = output_tokens(response.get("usage", {}))
                # A provider's final usage is authoritative.  The first visible
                # text timestamp excludes request setup and initial waiting time.
                if output_count and first_output_at is not None:
                    completed_at = now()
                    generation_ms = max(1, round((completed_at - first_output_at).total_seconds() * 1000))
                    event = record_performance(
                        db, run.owner_id, snapshot["model_record_id"], snapshot["provider_record_id"],
                        mode, output_count, generation_ms,
                    )
                    db.add(event)
                    performance = {
                        "output_tokens": output_count,
                        "generation_ms": generation_ms,
                        "tokens_per_second": event.data["tokens_per_second"],
                    }
                message_data = {
                    "role": "assistant",
                    "content": message["content"],
                    "run_id": run.id,
                    "artifacts": run.data.get("artifacts", []),
                }
                if performance:
                    message_data["performance"] = performance
                if run.data.get("auto_selection"):
                    message_data["auto_selection"] = {
                        **run.data["auto_selection"],
                        "model_record_id": snapshot["model_record_id"],
                        "model_id": snapshot["model_id"],
                    }
                db.add(Message(owner_id=run.owner_id, conversation_id=run.conversation_id, data=message_data))
                user_content = next((item.get("content", "") for item in reversed(run.data["history"]) if item.get("role") == "user"), "")
                if not run.data.get("temporary_mode"):
                    from mix_agent.memory.jobs import enqueue as enqueue_memory
                    enqueue_memory(db, run, user_content, message["content"], run.data.get("memory_trace_ids", []))
                conversation = db.get(Conversation, run.conversation_id)
                if conversation:
                    conversation.data = {**conversation.data, "last_message_at": now().isoformat()}
                emit(db, run.id, "message", {"content": message["content"], "usage": response.get("usage", {})})
                finish(db, run, "completed")
                return
    except asyncio.CancelledError:
        with SessionLocal() as db:
            run = db.get(Run, run_id)
            if run and run.status not in TERMINAL:
                finish(db, run, "cancelled", "ユーザーが停止しました")
        for kind in ("execution", "mcp"):
            try:
                await runner_request(kind, "/cancel", {"run_id": run_id}, timeout=5)
            except Exception:
                pass
    except Exception as exc:
        with SessionLocal() as db:
            run = db.get(Run, run_id)
            if run:
                finish(
                    db,
                    run,
                    "failed",
                    provider_failure_reason(exc, run.data.get("snapshot", {}).get("provider", {})),
                )
    finally:
        TASKS.pop(run_id, None)


async def scheduler():
    with SessionLocal() as db:
        for run in db.scalars(select(Run).where(Run.status == "running")):
            finish(db, run, "interrupted", "サーバーが再起動しました。結果不明の操作は自動再実行しません。")
        from mix_agent.schedules import reconcile
        reconcile(db, launch)
    sleep_interval = 2
    while True:
        try:
            from mix_agent.storage import backup

            if backup.ACTIVE:
                await asyncio.sleep(2)
                continue
            with SessionLocal() as db:
                from mix_agent.api.routes import purge_expired_conversations
                purge_expired_conversations(db)
                from mix_agent.schedules import tick
                tick(db, launch)
                for run in db.scalars(select(Run).where(Run.status.in_(["queued", "waiting_approval"]))):
                    if run.status == "queued":
                        launch(run.id)
                    else:
                        approval = db.scalar(
                            select(Approval).where(Approval.run_id == run.id, Approval.status == "pending")
                        )
                        if not approval or approval.expires.replace(tzinfo=UTC) < now():
                            launch(run.id)
            sleep_interval = 2
        except Exception:
            sleep_interval = min(30, sleep_interval * 2)
            import logging
            logging.getLogger(__name__).exception("scheduler tick failed")
        await asyncio.sleep(sleep_interval)
