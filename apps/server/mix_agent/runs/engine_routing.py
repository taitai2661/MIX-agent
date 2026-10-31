import asyncio


from mix_agent.context import budget as context_budget
from mix_agent.db.models import (
    Provider,
    now,
)
from mix_agent.providers.adapters import (
    is_nvidia_nim_function_not_found,
)
from mix_agent.providers.reasoning import resolve_reasoning
from mix_agent.routing import effective_capabilities, select_auto_model
from mix_agent.runs.state import transition_run


from mix_agent.runs.engine_support import (
    agent_interruption_reason,
    auto_retry_count,
    emit,
    finish,
    update,
)


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
        db,
        run.owner_id,
        routing["allowed_ids"],
        routing["content"],
        routing["mode"],
        routing["artifact_mimes"],
        routing["tools_required"],
        routing["context_parts"],
        routing["reserved_output_tokens"],
        request_key,
        routing["attachment_bytes"],
        excluded_model_ids=tuple(used_model_ids),
        prefer_other_provider_than=(
            provider_id if classification in {"rate_limit", "provider_5xx", "timeout", "incomplete"} else None
        ),
        priority=routing.get("priority", "balanced"),
    )
    if not model:
        return None
    provider = db.get(Provider, model.data["provider_id"])
    if not provider:
        return None
    try:
        reasoning = resolve_reasoning(
            provider.data["kind"],
            model.data["model_id"],
            effective_capabilities(model.data),
            snapshot["mode"],
            snapshot.get("model_settings", {}),
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
    attempts = [
        *attempts,
        {
            "model_record_id": previous_id,
            "model_id": snapshot["model_id"],
            "outcome": "provider_error",
            "error_type": type(error).__name__,
            "classification": classification,
        },
        {
            "model_record_id": model.id,
            "model_id": model.data["model_id"],
            "outcome": "retry",
            "retry_number": retry_number,
            "retry_limit": retry_limit,
        },
    ]
    selection["attempts"] = attempts
    # A retryable provider rejection produces no model output or tool work, so
    # it must not consume the conversation's execution-step budget.
    update(
        run,
        snapshot=updated_snapshot,
        auto_selection=selection,
        auto_model_history=[
            *run.data.get("auto_model_history", []),
            {
                "step": run.data.get("steps", 1),
                "from_model_record_id": previous_id,
                "to_model_record_id": model.id,
                "model_id": model.data["model_id"],
            },
        ],
        steps=max(0, run.data.get("steps", 0) - 1),
    )
    emit(
        db,
        run.id,
        "model_rerouted",
        {
            "from_model_record_id": previous_id,
            "to_model_record_id": model.id,
            "retry_number": retry_number,
            "retry_limit": retry_limit,
            "selection": selection,
        },
    )
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
        msg = body.get("message") or (
            body.get("error", {}).get("message") if isinstance(body.get("error"), dict) else None
        )
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


def _reevaluate_auto_model(db, run, snapshot, available):
    """Reconsider Auto only between provider calls; keep the Run's saved policy."""
    routing = run.data.get("auto_routing") or {}
    if run.data.get("requested_model_id") != "auto" or not routing.get(
        "dynamic_switching", snapshot.get("auto_dynamic_switching", True)
    ):
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
        db,
        run.owner_id,
        routing["allowed_ids"],
        content,
        snapshot["mode"],
        routing.get("artifact_mimes", []),
        tools_required,
        context_parts,
        routing.get("reserved_output_tokens", 4096),
        run.request_key,
        routing.get("attachment_bytes", 0),
        current_model_id=snapshot["model_record_id"],
        priority=routing.get("priority", snapshot.get("auto_priority", "balanced")),
    )
    if not model:
        return snapshot
    provider = db.get(Provider, model.data.get("provider_id"))
    if not provider or provider.owner_id != run.owner_id:
        return snapshot
    try:
        reasoning = resolve_reasoning(
            provider.data["kind"],
            model.data["model_id"],
            effective_capabilities(model.data),
            snapshot["mode"],
            snapshot.get("model_settings", {}),
        )
    except ValueError:
        return snapshot
    previous = snapshot["model_record_id"]
    selection["model_record_id"] = model.id
    selection["model_id"] = model.data["model_id"]
    selection["attempts"] = run.data.get("auto_selection", {}).get("attempts", [])
    if model.id != previous:
        snapshot = {
            **snapshot,
            "model_id": model.data["model_id"],
            "model_record_id": model.id,
            "provider": provider.data,
            "provider_record_id": provider.id,
            "reasoning": reasoning,
            "context_window_info": context_budget.resolve_window(model.data, snapshot),
        }
        changes = [
            *run.data.get("auto_model_history", []),
            {
                "step": run.data.get("steps", 0) + 1,
                "from_model_record_id": previous,
                "to_model_record_id": model.id,
                "model_id": model.data["model_id"],
            },
        ]
        update(run, snapshot=snapshot, auto_model_history=changes)
        emit(
            db,
            run.id,
            "model_rerouted",
            {
                "from_model_record_id": previous,
                "to_model_record_id": model.id,
                "selection": selection,
            },
        )
    update(run, auto_selection=selection)
    db.commit()
    return snapshot


def _handle_budget_limits(db, run, snapshot, steps, count, elapsed) -> "str | None":
    """Check step/tool-call budget limits and handle extensions or interruptions.

    Returns:
        ``'interrupted'``              – finish() was called; caller must ``return``.
        ``'budget_extension_pending'`` – transition made; caller must ``return``.
        ``None``                       – limits not reached; continue normally.
    """
    mode = snapshot["mode"]
    call_limit = snapshot.get("max_tool_calls", 50 if mode == "agent" else 8)
    step_limit = snapshot.get("max_steps", 8)
    seconds_limit = snapshot.get("max_seconds", 900)
    tool_limit_reached = count >= call_limit
    step_limit_reached = steps >= step_limit

    # Budget extension: pause and ask the user before stopping when policy allows.
    if snapshot.get("policy", {}).get("budget_extension") and (tool_limit_reached or step_limit_reached):
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
            emit(
                db,
                run.id,
                "budget_extension_requested",
                {
                    "tool_calls_used": count,
                    "tool_calls_limit": call_limit,
                    "steps_used": steps,
                    "steps_limit": step_limit,
                    "elapsed_seconds": int(elapsed),
                    "max_seconds": int(seconds_limit),
                    "extensions_used": extensions_used + 1,
                    "extensions_max": extensions_max,
                },
            )
            transition_run(db, run, "budget_extension_pending")
            db.commit()
            return "budget_extension_pending"

    if tool_limit_reached and snapshot.get("policy", {}).get("checkpointing"):
        finish(
            db,
            run,
            "interrupted",
            agent_interruption_reason(
                run, "Tool Call数の上限に到達しました。途中成果を確認して再開してください。"
            ),
        )
        return "interrupted"
    if step_limit_reached and snapshot.get("policy", {}).get("checkpointing"):
        finish(
            db,
            run,
            "interrupted",
            agent_interruption_reason(
                run, "モデル呼び出し回数の上限に到達しました。途中成果を確認して再開してください。"
            ),
        )
        return "interrupted"
    return None
