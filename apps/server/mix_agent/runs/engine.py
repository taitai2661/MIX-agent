import asyncio
from datetime import UTC, datetime

from sqlalchemy import select

from mix_agent.auth.security import read_secret
from mix_agent.context.types import ContextBudgetError
from mix_agent.db.models import (
    Approval,
    Conversation,
    Message,
    Model,
    Run,
    ScheduledRun,
    now,
)
from mix_agent.db.session import SessionLocal
from mix_agent.performance import record as record_performance
from mix_agent.providers.adapters import (
    is_nvidia_nim_chat_incompatible,
    is_nvidia_nim_function_not_found,
    is_retryable_provider_error,
)
from mix_agent.reliability import classify_failure, retry_after, usage_scope
from mix_agent.reliability import record as record_reliability
from mix_agent.runs.state import TERMINAL, can_transition, transition_run
from mix_agent.tools.execute import is_user_artifact, runner_request
from mix_agent.tools.registry import registry
from mix_agent.usage import record as record_usage


from mix_agent.runs.engine_context import _maybe_compact_context
from mix_agent.runs.engine_provider import _handle_provider_stream, _process_provider_response
from mix_agent.runs.engine_routing import (
    _handle_budget_limits,
    _reevaluate_auto_model,
    provider_failure_reason,
    reroute_after_provider_error,
)
from mix_agent.runs.engine_support import (
    PARALLEL_TOOL_LIMIT,
    _collect_memory_observations,
    _current_launch_epoch,
    _next_launch_epoch,
    agent_interruption_reason,
    emit,
    finish,
    is_parallel_safe,
    output_tokens,
    update,
)
from mix_agent.runs.engine_tools import (
    _process_pending_tools,
    complete_tool_call,
    execute_prepared_calls,
)

TASKS = {}


def launch(run_id):
    from mix_agent.storage import backup as backup_module

    # A backup/restore is replacing tables and runner state: never start new
    # work mid-restore. Queued runs are picked up again at the next scheduler
    # tick once ACTIVE clears.
    if backup_module.ACTIVE:
        return
    # Clean up completed tasks to prevent unbounded memory growth.
    done = [rid for rid, task in list(TASKS.items()) if task.done()]
    for rid in done:
        TASKS.pop(rid, None)
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

    def _forget(completed):
        # Remove the finished task immediately instead of waiting for the next
        # launch, so a long-lived process does not retain completed Run state.
        if TASKS.get(run_id) is completed:
            TASKS.pop(run_id, None)

    task.add_done_callback(_forget)
    TASKS[run_id] = task


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
                    update(
                        run,
                        browser_pause_requested=False,
                        browser_manual_active=True,
                        browser_paused_at=now().isoformat(),
                    )
                    transition_run(db, run, "paused")
                    db.commit()
                    return
                elapsed = (now() - run.created_at.replace(tzinfo=UTC)).total_seconds()
                if elapsed >= snapshot.get("max_seconds", 900):
                    if snapshot.get("policy", {}).get("checkpointing"):
                        finish(
                            db,
                            run,
                            "interrupted",
                            agent_interruption_reason(
                                run,
                                "実行時間の上限に到達しました。途中成果を確認して、必要なら予算を調整して再開してください。",
                            ),
                        )
                    else:
                        finish(
                            db,
                            run,
                            "completed",
                            "このモードの実行時間上限に到達したため、取得済みの結果で終了しました。長い作業には長作業モードを使用してください。",
                        )
                    return

                # --- Phase 1: collect and dispatch pending tool calls ----------
                prepared = _process_pending_tools(db, run, snapshot)
                # _process_pending_tools returns [] when waiting_approval was
                # detected (transition + commit already done inside the helper).
                if run.status == "waiting_approval":
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
                    update(
                        run,
                        browser_pause_requested=False,
                        browser_manual_active=True,
                        browser_paused_at=now().isoformat(),
                    )
                    transition_run(db, run, "paused")
                    db.commit()
                    return

                # Periodic durable checkpoint so resume can pick a recent step.
                from mix_agent.runs import checkpoints as run_checkpoints

                if snapshot.get("policy", {}).get("checkpointing") and run_checkpoints.should_checkpoint(
                    snapshot, int(run.data.get("steps", 0))
                ):
                    try:
                        checkpoint = run_checkpoints.save(db, run, trigger="interval")
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
                                            {
                                                "code": f["code"],
                                                "severity": f.get("severity"),
                                                "tool_id": f.get("tool_id"),
                                                "count": f.get("count"),
                                            }
                                            for f in findings
                                        ],
                                    )
                                    emit(
                                        db,
                                        run.id,
                                        "stagnation_detected",
                                        {
                                            "findings": [
                                                {
                                                    "code": f["code"],
                                                    "severity": f.get("severity"),
                                                    "tool_id": f.get("tool_id"),
                                                    "count": f.get("count"),
                                                }
                                                for f in findings
                                            ],
                                            "step": current_step,
                                        },
                                    )
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

                # --- Phase 2: budget limit checks ----------------------------
                budget_outcome = _handle_budget_limits(db, run, snapshot, steps, count, elapsed)
                if budget_outcome in ("interrupted", "budget_extension_pending"):
                    return
                # Re-compute available after limit checks (checkpointing path
                # may not have cleared available when limits are reached without
                # the checkpointing policy; mirror original logic exactly).
                if tool_limit_reached:
                    available = []
                if step_limit_reached:
                    available = []

                snapshot = _reevaluate_auto_model(db, run, snapshot, available)
                provider = snapshot["provider"]
                # Credentials are resolved at use time, never copied into run snapshots.
                key = read_secret(db, provider.get("secret_id"))
                # Model-aware pre-send budget check with progressive compaction.
                try:
                    send_history = await _maybe_compact_context(db, run, snapshot, provider, key)
                except ContextBudgetError as exc:
                    finish(db, run, "interrupted", str(exc))
                    return
                emit(
                    db,
                    run.id,
                    "model_started",
                    {
                        "step": steps + 1,
                        "mode": mode,
                        "model_id": snapshot.get("model_id"),
                        "model_record_id": snapshot.get("model_record_id"),
                        "provider_id": snapshot.get("provider_record_id"),
                    },
                )
                update(run, steps=steps + 1)
                db.commit()

                # --- Phase 3: provider streaming -----------------------------
                first_output_at = None
                provider_started_at = now()
                stream_state: dict = {}
                try:
                    (
                        response,
                        _last_event,
                        first_output_at,
                        provider_started_at,
                    ) = await _handle_provider_stream(
                        db,
                        run,
                        snapshot,
                        send_history,
                        available,
                        tool_limit_reached,
                        step_limit_reached,
                        elapsed,
                        mode,
                        state=stream_state,
                    )
                except asyncio.CancelledError:
                    raise
                except Exception as exc:
                    # A provider that failed *after* the first token already
                    # showed the user something. The stream helper records
                    # that in `stream_state`, because the exception path
                    # never receives its return value — without it the
                    # retry and reroute paths would replay the request and
                    # the user would see the answer twice.
                    first_output_at = stream_state.get("first_output_at", first_output_at)
                    retryable = (
                        is_retryable_provider_error(exc)
                        or is_nvidia_nim_function_not_found(provider, exc)
                        or is_nvidia_nim_chat_incompatible(provider, snapshot["model_id"], exc)
                    )
                    classification = classify_failure(exc)
                    retry_until = retry_after(exc)
                    if run.data.get("requested_model_id") == "auto" and snapshot.get("provider_record_id"):
                        record_reliability(
                            db,
                            run.owner_id,
                            snapshot["model_record_id"],
                            snapshot["provider_record_id"],
                            usage_scope(mode, bool(run.data.get("auto_routing", {}).get("tools_required"))),
                            "failure",
                            classification,
                            retry_until,
                            profile=run.data.get("auto_selection", {}).get("profile"),
                            required_tokens=run.data.get("auto_selection", {}).get("required_tokens"),
                        )
                    rerouted = (
                        await reroute_after_provider_error(
                            db, run, run.request_key, exc, classification, retry_until
                        )
                        if run.data.get("requested_model_id") == "auto"
                        and retryable
                        and first_output_at is None
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
                    if transient and retries < 4 and retry_budget > 0:
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

                model_record = (
                    db.get(Model, snapshot["model_record_id"]) if snapshot.get("model_record_id") else None
                )
                record_usage(
                    db,
                    run.owner_id,
                    model_record.data if model_record else {},
                    snapshot.get("provider_record_id"),
                    mode,
                    response.get("usage", {}),
                    run_id=run.id,
                    input_estimate=(run.data.get("context_trace") or {}).get("estimated_input_tokens"),
                    model_record_id=snapshot.get("model_record_id"),
                )
                if run.data.get("requested_model_id") == "auto" and snapshot.get("provider_record_id"):
                    completed_at = now()
                    first_output_at = first_output_at or completed_at
                    record_reliability(
                        db,
                        run.owner_id,
                        snapshot["model_record_id"],
                        snapshot["provider_record_id"],
                        usage_scope(mode, bool(run.data.get("auto_routing", {}).get("tools_required"))),
                        "success",
                        first_output_ms=max(
                            1, round((first_output_at - provider_started_at).total_seconds() * 1000)
                        ),
                        completion_ms=max(
                            1, round((completed_at - provider_started_at).total_seconds() * 1000)
                        ),
                        output_tokens=output_tokens(response.get("usage", {})),
                        profile=run.data.get("auto_selection", {}).get("profile"),
                        required_tokens=run.data.get("auto_selection", {}).get("required_tokens"),
                    )

                # --- Phase 4: process provider response ----------------------
                outcome = _process_provider_response(
                    db,
                    run,
                    snapshot,
                    response,
                    available,
                    mode,
                    steps,
                    count,
                    call_limit,
                )
                if outcome == "continue":
                    continue
                if outcome == "interrupted":
                    return

                # outcome == "completed": finalise the run.
                # The step helper may have replaced a non-user-facing answer
                # with the safe fallback, and that substitution lives in the
                # history — not in the raw provider response. Reading the
                # final answer from the history keeps the persisted message
                # and the answer the user actually receives the same text.
                message = response["message"]
                answered = [item for item in run.data.get("history", []) if item.get("role") == "assistant"]
                if answered:
                    message = answered[-1]
                performance = None
                output_count = output_tokens(response.get("usage", {}))
                # A provider's final usage is authoritative.  The first visible
                # text timestamp excludes request setup and initial waiting time.
                if output_count and first_output_at is not None:
                    completed_at = now()
                    generation_ms = max(1, round((completed_at - first_output_at).total_seconds() * 1000))
                    first_output_ms = max(
                        1, round((first_output_at - provider_started_at).total_seconds() * 1000)
                    )
                    event = record_performance(
                        db,
                        run.owner_id,
                        snapshot["model_record_id"],
                        snapshot["provider_record_id"],
                        mode,
                        output_count,
                        generation_ms,
                        first_output_ms,
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
                        "model_name": (selected_model.data.get("name") or snapshot["model_id"])
                        if selected_model
                        else snapshot["model_id"],
                    }
                db.add(Message(owner_id=run.owner_id, conversation_id=run.conversation_id, data=message_data))
                user_content = next(
                    (
                        item.get("content", "")
                        for item in reversed(run.data["history"])
                        if item.get("role") == "user"
                    ),
                    "",
                )
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
                                plan = (
                                    evaluate(
                                        db,
                                        run.owner_id,
                                        task_id=run.id,
                                        observation=observation,
                                        explicit_user=bool(observation.get("explicit_user")),
                                    ).get("plan")
                                    or []
                                )
                                apply_plan(db, run.owner_id, plan, run_id=run.id)
                    except Exception:  # noqa: BLE001, S110 - synchronous memory writes must never block completion
                        # begin_nested() rolls back only the memory SAVEPOINT.
                        pass
                    from mix_agent.memory.jobs import enqueue as enqueue_memory

                    enqueue_memory(
                        db, run, user_content, message["content"], run.data.get("memory_trace_ids", [])
                    )
                conversation = db.get(Conversation, run.conversation_id)
                if conversation:
                    conversation.data = {**conversation.data, "last_message_at": now().isoformat()}
                emit(
                    db, run.id, "message", {"content": message["content"], "usage": response.get("usage", {})}
                )
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
        except Exception:  # noqa: BLE001, S110 - the browser may already be closed
            pass


def _scheduler_maintenance(launch) -> float:
    """Run the synchronous part of one scheduler tick.

    Everything here uses the sync SQLAlchemy session, so the caller executes it
    through ``asyncio.to_thread``. ``launch`` must therefore be a plain
    callable (the caller passes ``list.append``); the real ``launch()`` creates
    an asyncio task and has to run back on the event loop.
    """
    delay = 30.0
    with SessionLocal() as db:
        from mix_agent.auth.security import purge_expired_sessions
        from mix_agent.schedules import tick
        from mix_agent.storage.cleanup import (
            purge_expired_conversations,
            purge_old_run_history,
        )

        if purge_expired_sessions(db):
            db.commit()
        purge_old_run_history(db)
        purge_expired_conversations(db)
        tick(db, launch)
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
                delay = min(
                    delay,
                    max(0.05, (approval.expires.replace(tzinfo=UTC) - current).total_seconds()),
                )
        for scheduled in db.scalars(select(ScheduledRun).where(ScheduledRun.status == "retrying")):
            try:
                retry_at = datetime.fromisoformat(scheduled.data.get("retry_at", ""))
                delay = min(delay, max(0.05, (retry_at - current).total_seconds()))
            except (TypeError, ValueError):
                pass
    return delay


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
        except Exception:  # noqa: BLE001, S110 - the browser may already be closed
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
                # The whole maintenance pass is synchronous DB work; run it in
                # the threadpool so a slow query cannot stall every SSE stream
                # in this single-process deployment. `launch` is collected as
                # IDs and replayed on the loop, because it creates a task.
                launched: list[str] = []
                delay = await asyncio.to_thread(_scheduler_maintenance, launched.append)
                for run_id in launched:
                    launch(run_id)
                # Confirm unknown/stale model capabilities in the background so
                # Auto does not depend on a user-triggered check. A hard timeout
                # keeps a slow provider from stalling the tick.
                from mix_agent import config
                from mix_agent.providers.probing import probe_due_models

                try:
                    with SessionLocal() as db:
                        await asyncio.wait_for(
                            probe_due_models(db), timeout=config.PROBE_TICK_TIMEOUT_SECONDS
                        )
                except Exception:
                    import logging

                    logging.getLogger(__name__).exception("capability probe pass failed")
        except Exception:
            delay = 30.0
            import logging

            logging.getLogger(__name__).exception("scheduler tick failed")
        seen = await wakeup.wait(seen, timeout=delay)
