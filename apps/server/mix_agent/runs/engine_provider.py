import asyncio
import json

from sqlalchemy import select

from mix_agent.db.models import (
    Message,
    ToolCall,
    now,
)
from mix_agent.providers.adapters import (
    Adapter,
    ProviderIncompleteResponseError,
)
from mix_agent.tools.execute import is_user_artifact
from mix_agent.tools.registry import fingerprint


from mix_agent.runs.engine_support import (
    STREAM_TEXT_FLUSH_SECONDS,
    _reject_tool_call,
    activity_summary,
    agent_completion_issues,
    agent_interruption_reason,
    answer_fallback,
    emit,
    finish,
    flush_stream_text,
    is_user_facing_answer,
    update,
    wait_for_provider_slot,
)
from mix_agent.runs.engine_tools import complete_tool_call


async def _handle_provider_stream(
    db,
    run,
    snapshot,
    send_history,
    available,
    tool_limit_reached,
    step_limit_reached,
    elapsed,
    mode,
    state=None,
):
    """Stream one provider turn and return the final response event plus timing metadata.

    Handles text buffering, per-provider rate limiting, pinned-model transient
    retries, and Auto-model rerouting on retryable errors.  Exceptions that
    cannot be recovered are re-raised so the caller's outer handler decides
    how to finish the run.

    ``state`` is an optional dict the caller passes in; it receives
    ``first_output_at`` as soon as the first text token is visible, so the
    caller's error handler still knows that output was already shown when
    the provider fails mid-stream.

    Returns:
        ``(response, last_event, first_output_at, provider_started_at)``
        where ``response`` is the provider's final ``response`` event dict,
        ``last_event`` is the same object (kept for symmetry with earlier
        refactors), ``first_output_at`` is the datetime of the first streamed
        text token (or None), and ``provider_started_at`` is when the request
        was dispatched.
    """
    provider = snapshot["provider"]
    seconds_limit = snapshot.get("max_seconds", 900)
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

    from mix_agent.auth.security import read_secret as _read_secret

    key = _read_secret(db, provider.get("secret_id"))

    try:
        response = None
        async with asyncio.timeout(max(1, seconds_limit - elapsed)):
            await wait_for_provider_slot({**provider, "id": snapshot.get("provider_record_id")})
            async for event in Adapter(provider, key).stream(
                snapshot["model_id"],
                (
                    [
                        *send_history,
                        {
                            "role": "user",
                            "content": "The step or tool-call limit for this response mode has been reached. Do not request more tools; answer the original request using the available information and recommend Long work mode if sustained work remains.",
                        },
                    ]
                    if tool_limit_reached or step_limit_reached
                    else send_history
                ),
                available,
                mode,
                model_settings,
            ):
                if event["kind"] == "response":
                    if flush_stream_text(db, run.id, buffered_text):
                        last_text_flush_at = asyncio.get_running_loop().time()
                    response = event
                elif event["kind"] == "text":
                    if event.get("text") and first_output_at is None:
                        first_output_at = now()
                        if state is not None:
                            # Hand the visible-output state to the caller: a
                            # provider that fails *after* the first token has
                            # already shown the user something, and that must
                            # survive the exception.
                            state["first_output_at"] = first_output_at
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
    except asyncio.CancelledError:
        flush_stream_text(db, run.id, buffered_text)
        raise
    except Exception:
        # A failed stream must still expose every fragment the
        # provider already produced before reporting its outcome.
        flush_stream_text(db, run.id, buffered_text)
        raise

    return response, response, first_output_at, provider_started_at


def _process_provider_response(db, run, snapshot, response, available, mode, steps, count, call_limit) -> str:
    """Persist the provider response, create ToolCall rows, and evaluate the answer.

    Handles:
    - Appending the assistant message to history
    - Creating ToolCall rows for every requested tool call
    - Enforcing the per-run call-limit cap
    - Detecting non-user-facing answers and injecting repair turns
    - Running Agent completion verification (plan / evidence gates)

    Returns:
        ``'continue'``    – the loop should continue to the next iteration.
        ``'completed'``   – finish(completed) was called; caller must ``return``.
        ``'interrupted'`` – finish(interrupted) was called; caller must ``return``.
    """
    history = run.data["history"]
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
                    db,
                    run,
                    c,
                    available,
                    error="The requested tool is not available in this Run.",
                    code="tool_unavailable",
                )
                continue
            try:
                args = json.loads(c["arguments"]) if isinstance(c["arguments"], str) else c["arguments"]
            except (TypeError, ValueError):
                _reject_tool_call(
                    db,
                    run,
                    c,
                    available,
                    error="The tool arguments were not valid JSON. Send a JSON object matching the tool schema.",
                    code="tool_arguments_invalid_json",
                )
                continue
            if not isinstance(args, dict):
                _reject_tool_call(
                    db,
                    run,
                    c,
                    available,
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
                    db,
                    run,
                    call,
                    tool,
                    {
                        "error": "Tool call limit reached",
                        "code": "tool_call_limit_reached",
                        "type": "tool_call_limit_reached",
                    },
                )
        update(run, tool_count=min(call_limit, count + len(tool_calls)))
        if snapshot.get("policy", {}).get("checkpointing"):
            update(
                run,
                checkpoint={
                    "status": "working",
                    "steps": steps + 1,
                    "tool_count": min(call_limit, count + len(tool_calls)),
                },
            )
            # Persist a durable checkpoint at every step boundary so
            # the user can pick the most recent working state.
            try:
                from mix_agent.runs import checkpoints as run_checkpoints

                checkpoint = run_checkpoints.save(db, run, trigger="working")
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
        db.commit()
        return "continue"
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
            return "continue"
        message = {"role": "assistant", "content": answer_fallback(run.data["history"])}
        update(run, history=[*history, message])
        update(
            run,
            answer_evaluation={
                "status": "needs_review",
                "reason": "自動補足で終了しました。依頼の達成を確認してください。",
            },
        )
    else:
        update(
            run,
            answer_evaluation={
                "status": "provided",
                "reason": "ユーザー向けの最終回答を確認しました。内容の正しさは自動判定していません。",
            },
        )
    issues = agent_completion_issues(
        run, list(db.scalars(select(ToolCall).where(ToolCall.run_id == run.id).order_by(ToolCall.created_at)))
    )
    if issues:
        repairs = run.data.get("agent_completion_repairs", 0)
        if run.data.get("agent_plan") and repairs < 1 and steps < snapshot.get("max_steps", 8):
            update(
                run,
                agent_completion_repairs=repairs + 1,
                history=[
                    *run.data["history"],
                    {
                        "role": "user",
                        "content": "The task is not yet verified:\n- "
                        + "\n- ".join(issues)
                        + "\nContinue the work, update the plan with remaining tasks and verification, then answer. If blocked, report the blocker and remaining work.",
                    },
                ],
            )
            db.commit()
            return "continue"
        # Keep the model's useful partial answer visible even when
        # the Agent cannot satisfy the verification gate. The Run
        # remains interrupted and the answer is explicitly marked
        # for review; silently discarding it made a successful
        # final response disappear because of a bookkeeping gate.
        update(
            run,
            answer_evaluation={
                "status": "needs_review",
                "reason": "作業結果を自動確認できませんでした。回答と残作業を確認してください。",
            },
        )
        db.add(
            Message(
                owner_id=run.owner_id,
                conversation_id=run.conversation_id,
                data={
                    "role": "assistant",
                    "content": message["content"],
                    "run_id": run.id,
                    "artifacts": [a for a in run.data.get("artifacts", []) if is_user_artifact(a)],
                    "answer_evaluation": run.data["answer_evaluation"],
                },
            )
        )
        finish(db, run, "interrupted", agent_interruption_reason(run, issues[0]))
        return "interrupted"
    return "completed"
