import asyncio
import json
from datetime import UTC, timedelta

from sqlalchemy import select

from mix_agent import todos as conversation_todos
from mix_agent.context import task_state as context_task_state
from mix_agent.context.references import (
    DEFAULT_TOOL_INLINE_LIMIT,
    tool_envelope_text,
    tool_ref_message,
)
from mix_agent.db.models import (
    Approval,
    Conversation,
    Run,
    Settings,
    ToolCall,
    now,
)
from mix_agent.db.session import SessionLocal
from mix_agent.runs.state import transition_run
from mix_agent.tools.execute import execute, is_user_artifact
from mix_agent.tools.registry import call_scope, fingerprint, permission, registry


from mix_agent.runs.engine_support import (
    SAME_TOOL_ARGUMENT_LIMIT,
    activity_result,
    activity_summary,
    completed_tool_attempts,
    emit,
    model_tool_result,
    tool_attempt_key,
    update,
)


def complete_tool_call(db, run, call, current, result):
    """Persist one result in provider-call order after execution has settled."""
    if isinstance(result, dict) and call.data.get("tool_id", "").startswith("browser_"):
        frame_bytes = result.pop("frame_base64", None)
        result.pop("frame_unavailable", None)
        if frame_bytes:
            try:
                import base64

                from mix_agent.tools.execute import save_artifact as _save_artifact

                frame = _save_artifact(
                    db,
                    run.owner_id,
                    base64.b64decode(frame_bytes),
                    "browser-frame.png",
                    "image/png",
                    kind="browser-frame",
                )
                frames = list(run.data.get("browser_frames") or [])
                from urllib.parse import urlsplit, urlunsplit

                address = urlsplit(result.get("url", ""))
                visible_url = urlunsplit((address.scheme, address.netloc, address.path, "", ""))[:2048]
                frames.append(
                    {
                        "artifact_id": frame["artifact_id"],
                        "call_id": call.id,
                        "url": visible_url,
                        "tool": call.data["tool_id"],
                    }
                )
                if len(frames) > 30:
                    from mix_agent import config
                    from mix_agent.db.models import Artifact

                    for old in frames[:-30]:
                        old_id = old["artifact_id"]
                        old_row = db.get(Artifact, old_id)
                        if old_row:
                            (config.ARTIFACTS / old_id).unlink(missing_ok=True)
                            db.delete(old_row)
                    frames = frames[-30:]
                update(run, browser_frames=frames)
                emit(db, run.id, "browser_frame", frames[-1])
            except Exception:  # noqa: BLE001, S110 - a screenshot failure must not fail the browser operation
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
                db,
                run.owner_id,
                content.encode("utf-8"),
                "tool-result.json",
                "application/json",
                kind="context-tool-output",
            )
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            artifact_info = None
    if is_long:
        tool_ref = (artifact_info or {}).get("artifact_id")
        entry = tool_ref_message(
            call.data["provider_call_id"],
            call.data["name"],
            short_summary[:2000],
            tool_ref,
            True,
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
        emit(
            db,
            run.id,
            "tool_started",
            {
                "id": call.id,
                "name": current["model_name"],
                "activity": activity_summary(current["id"], call.data["arguments"]),
            },
        )
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


def _process_pending_tools(db, run, snapshot) -> list:
    """Collect pending/waiting_approval ToolCalls and prepare them for execution.

    Evaluates each pending ToolCall against the current tool registry, retry
    budget and permission gate. Calls that need human approval transition to
    waiting_approval and short-circuit by returning an empty list (the caller
    must persist the transition and return).

    Returns a list of ``(call, current, result)`` triples:
    - ``current`` is the live tool definition (may be None when the definition
      changed since the call was created).
    - ``result`` is pre-filled when the call should be completed immediately
      (loop-detected, denied, …) or None when it should be executed normally.
    """
    calls = list(
        db.scalars(
            select(ToolCall)
            .where(ToolCall.run_id == run.id, ToolCall.status.in_(["pending", "waiting_approval"]))
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
            result = {
                "error": "Tool definition changed",
                "code": "tool_definition_changed",
                "type": "tool_definition_changed",
            }
        else:
            key = tool_attempt_key(current["id"], call.data["arguments"])
            if attempt_counts[key] >= SAME_TOOL_ARGUMENT_LIMIT:
                result = {
                    "error": "Tool loop detected",
                    "code": "tool_loop_detected",
                    "type": "tool_loop_detected",
                }
            else:
                attempt_counts[key] += 1
                try:
                    decision = permission(db, run, current, call.data["arguments"])
                except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
                    decision = "deny"
                if decision == "deny":
                    result = {"error": "Tool denied", "code": "tool_denied", "type": "tool_denied"}
                elif decision == "ask" and run.data.get("scheduled_run_id"):
                    result = {
                        "error": "Scheduled runs require an always-allowed tool",
                        "code": "tool_denied",
                        "type": "tool_denied",
                    }
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
                    if approval.expires.replace(tzinfo=UTC) < now() and approval.status == "pending":
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
        return []  # Signal to the caller that it should return immediately.
    return prepared
