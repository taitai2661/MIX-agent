import asyncio
import base64
import json
from datetime import UTC, datetime
from urllib.parse import urlsplit

from fastapi import APIRouter, Depends, HTTPException, Request
from fastapi.responses import FileResponse, StreamingResponse
from sqlalchemy import select
from sqlalchemy.exc import IntegrityError

from mix_agent import config
from mix_agent.api.common import (
    audit,
    context,
    own,
    public,
)
from mix_agent.api.schemas import (
    BudgetExtensionDecisionInput,
    DecisionInput,
    ResumeInput,
    RunView,
)
from mix_agent.auth.security import (
    authenticate,
)
from mix_agent.db.models import (
    Approval,
    Artifact,
    Event,
    Permission,
    Run,
    ToolCall,
    now,
)
from mix_agent.db.session import SessionLocal
from mix_agent.runs.engine import TASKS, launch
from mix_agent.runs.state import transition_run
from mix_agent.tools.execute import runner_request, save_artifact
from mix_agent.tools.registry import fingerprint, registry

router = APIRouter(prefix="/api/v1")

# Backward-compatible re-exports for external callers (engine.py, scheduler, etc.)


@router.get("/runs/{key}", response_model=RunView)
def get_run(key: str, ctx=Depends(context)):
    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    snapshot = run.data["snapshot"]
    elapsed = max(0, int((now() - run.created_at.replace(tzinfo=UTC)).total_seconds()))
    if (
        run.status in ("paused", "waiting_approval")
        and run.data.get("browser_manual_active")
        and run.data.get("browser_paused_at")
    ):
        elapsed = max(
            0, elapsed - int((now() - datetime.fromisoformat(run.data["browser_paused_at"])).total_seconds())
        )
    budget = {k: snapshot.get(k) for k in ("max_seconds", "max_steps", "max_tool_calls")}
    remaining = {
        "max_seconds": max(0, budget["max_seconds"] - elapsed) if budget["max_seconds"] is not None else None,
        "max_steps": max(0, budget["max_steps"] - run.data.get("steps", 0))
        if budget["max_steps"] is not None
        else None,
        "max_tool_calls": max(0, budget["max_tool_calls"] - run.data.get("tool_count", 0))
        if budget["max_tool_calls"] is not None
        else None,
    }
    policy = snapshot.get("policy") or {}
    from mix_agent.runs import checkpoints as run_checkpoints

    checkpoints = (
        run_checkpoints.list_for_run(db, run.id)
        if run.status in {"interrupted", "failed", "paused", "budget_extension_pending", "completed"}
        else []
    )
    return {
        "id": run.id,
        "status": run.status,
        "browser_manual_active": bool(run.data.get("browser_manual_active")),
        "browser_pause_requested": bool(run.data.get("browser_pause_requested")),
        "reason": run.data.get("reason"),
        "steps": run.data.get("steps", 0),
        "tool_count": run.data.get("tool_count", 0),
        "mode": snapshot.get("mode", "chat"),
        "policy": policy,
        "budget": budget,
        "remaining": remaining,
        "approvals": [
            {**public(a), "status": a.status}
            for a in db.scalars(select(Approval).where(Approval.run_id == key))
        ],
        "context_summary": run.data.get("summary") if (run.data.get("summary") or {}).get("text") else None,
        "answer_evaluation": run.data.get("answer_evaluation"),
        "budget_extension_request": run.data.get("budget_extension_request"),
        "budget_extensions_used": int(run.data.get("budget_extensions_used", 0)),
        "budget_extensions_max": int(policy.get("max_budget_extensions", 0)),
        "stagnation": run.data.get("stagnation_findings"),
        "checkpoint_resumed_from": run.data.get("checkpoint_resumed_from"),
        "checkpoints": checkpoints,
        "context_trace": run.data.get("context_trace"),
    }


@router.get("/runs/{key}/checkpoints")
def list_checkpoints(key: str, ctx=Depends(context)):
    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    from mix_agent.runs import checkpoints as run_checkpoints

    return {"checkpoints": run_checkpoints.list_for_run(db, run.id)}


@router.get("/runs/{key}/events")
async def run_events(key: str, request: Request, after: int = 0):
    # Authenticate with a short-lived session.  Keeping the request-scoped
    # dependency session alive for the lifetime of an SSE stream would pin a
    # connection from the small SQLAlchemy pool per connected browser tab.
    with SessionLocal() as auth_db:
        session = authenticate(request, auth_db)
        own(auth_db, Run, key, session.owner_id)
    try:
        cursor = max(after, int(request.headers.get("last-event-id", "0")))
    except ValueError:
        raise HTTPException(422)
    from mix_agent.wakeups import run_events

    wakeup = run_events.for_run(key)

    async def stream():
        nonlocal cursor
        while not await request.is_disconnected():
            with SessionLocal() as session:
                events = list(
                    session.scalars(
                        select(Event)
                        .where(Event.run_id == key, Event.sequence > cursor)
                        .order_by(Event.sequence)
                        .limit(200)
                    )
                )
                run = session.get(Run, key)
                # The run can be deleted between the authenticated request and
                # this poll; end the stream instead of dereferencing None.
                if run is None:
                    yield "event: done\ndata: {}\n\n"
                    return
                status = run.status
                for event in events:
                    cursor = event.sequence
                    yield f"id: {cursor}\nevent: activity\ndata: {json.dumps({'kind': event.kind, **event.data}, ensure_ascii=False)}\n\n"
                if status in ("completed", "failed", "cancelled", "interrupted") and len(events) < 200:
                    yield "event: done\ndata: {}\n\n"
                    return
            # Keep proxies from closing an idle SSE stream without polling the DB.
            seen = wakeup.revision
            changed = await wakeup.wait(seen, timeout=15)
            if changed == seen:
                yield ": heartbeat\n\n"

    return StreamingResponse(
        stream(),
        media_type="text/event-stream",
        headers={"Cache-Control": "no-cache", "X-Accel-Buffering": "no"},
    )


@router.post("/runs/{key}/cancel")
async def cancel(key: str, ctx=Depends(context)):
    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    if run.status not in ("completed", "failed", "cancelled"):
        transition_run(db, run, "cancelled")
        db.commit()
        if key in TASKS:
            TASKS[key].cancel()
        for kind in ("execution", "mcp"):
            try:
                await runner_request(kind, "/cancel", {"run_id": key}, timeout=5)
            except Exception:  # noqa: BLE001, S110 - intentionally classified; never leak raw details
                pass
        try:
            await runner_request("browser", "/browser-close", {"run_id": key}, timeout=5)
        except Exception:  # noqa: BLE001, S110 - the browser may already be closed
            pass
    return {"ok": True}


def _record_browser_frame(db, run, frame, tool="manual"):
    encoded = frame.pop("frame_base64", None)
    if not encoded:
        return None

    artifact = save_artifact(
        db, run.owner_id, base64.b64decode(encoded), "browser-frame.png", "image/png", kind="browser-frame"
    )
    frames = list(run.data.get("browser_frames") or [])
    address = urlsplit(frame.get("url", ""))
    visible_url = address._replace(query="", fragment="").geturl()[:2048]
    entry = {"artifact_id": artifact["artifact_id"], "url": visible_url, "tool": tool}
    frames.append(entry)
    for old in frames[:-30]:
        old_id = old["artifact_id"]
        old_row = db.get(Artifact, old_id)
        if old_row:
            (config.ARTIFACTS / old_id).unlink(missing_ok=True)
            db.delete(old_row)
    run.data = {**run.data, "browser_frames": frames[-30:]}
    from mix_agent.runs.engine import emit

    emit(db, run.id, "browser_frame", entry)
    return entry


@router.get("/runs/{key}/browser-frames")
def browser_frames(key: str, ctx=Depends(context)):
    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    return run.data.get("browser_frames", [])


@router.get("/runs/{key}/browser-frames/{artifact_id}")
def browser_frame_image(key: str, artifact_id: str, ctx=Depends(context)):
    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    if not any(item.get("artifact_id") == artifact_id for item in run.data.get("browser_frames", [])):
        raise HTTPException(404, "画面が見つかりません")
    row = own(db, Artifact, artifact_id, s.owner_id)
    if row.data.get("kind") != "browser-frame":
        raise HTTPException(404, "画面が見つかりません")
    return FileResponse(
        config.ARTIFACTS / artifact_id,
        media_type="image/png",
        headers={
            "Content-Disposition": "inline",
            "X-Content-Type-Options": "nosniff",
            "Cache-Control": "private, no-store",
            "Content-Security-Policy": "default-src 'none'; sandbox",
        },
    )


@router.post("/runs/{key}/browser-pause")
def browser_pause(key: str, ctx=Depends(context)):
    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    if not run.data.get("browser_frames"):
        raise HTTPException(409, "操作できるブラウザ画面がありません")
    if run.status == "running":
        run.data = {**run.data, "browser_pause_requested": True}
    elif run.status == "waiting_approval":
        pending_browser = any(
            str(item.data.get("tool", "")).startswith("browser_")
            for item in db.scalars(
                select(Approval).where(Approval.run_id == key, Approval.status == "pending")
            )
        )
        if not pending_browser:
            raise HTTPException(409, "Browser操作の承認待ちではありません")
        run.data = {**run.data, "browser_manual_active": True, "browser_paused_at": now().isoformat()}
    elif run.status != "paused":
        raise HTTPException(409, "この実行は手動操作を開始できません")
    db.commit()
    return {"ok": True}


@router.post("/runs/{key}/browser-manual")
async def browser_manual(key: str, request: Request, ctx=Depends(context)):
    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    if run.status not in ("paused", "waiting_approval") or not run.data.get("browser_manual_active"):
        raise HTTPException(409, "手動操作は有効ではありません")
    body = await request.json()
    action = body.get("action")
    if action not in ("frame", "click", "type", "key", "scroll"):
        raise HTTPException(422, "操作が無効です")
    # Never save input text, including passwords, to a Run, Event or audit row.
    payload = {"run_id": key, "action": action}
    for field in ("x", "y", "dy", "key", "text"):
        if field in body:
            payload[field] = body[field]
    from mix_agent.tools.execute import runner_request

    frame = await runner_request("browser", "/browser-manual", payload)
    entry = None
    if action != "frame":
        try:
            entry = _record_browser_frame(db, run, frame)
        except Exception:  # noqa: BLE001 - the observation may already have been stored; never imply failure
            db.rollback()
            # The browser action may already have completed. Do not imply failure
            # or retry it merely because its observation could not be stored.
    db.commit()
    return {"frame": entry or (run.data.get("browser_frames") or [None])[-1]}


@router.post("/runs/{key}/browser-return")
async def browser_return(key: str, ctx=Depends(context)):
    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    if run.status not in ("paused", "waiting_approval") or not run.data.get("browser_manual_active"):
        raise HTTPException(409, "手動操作は有効ではありません")
    data = {**run.data, "browser_manual_active": False}
    paused_at = datetime.fromisoformat(data.pop("browser_paused_at"))
    elapsed = max(0, int((now() - paused_at).total_seconds() + 0.999))
    data["snapshot"] = {
        **data["snapshot"],
        "max_seconds": (data["snapshot"].get("max_seconds") or 900) + elapsed,
    }
    if run.status == "paused":
        data["history"] = [
            *data.get("history", []),
            {
                "role": "user",
                "content": "ブラウザで手動操作が行われました。現在のページを確認してから続けてください。",
            },
        ]
        run.data = data
        transition_run(db, run, "queued")
        db.commit()
        pending = TASKS.get(key)
        if pending and not pending.done():
            try:
                await asyncio.wait_for(asyncio.shield(pending), timeout=2)
            except (TimeoutError, asyncio.CancelledError):
                pending.add_done_callback(lambda _: launch(key))
            except Exception:  # noqa: BLE001, S110 - the pending run may already have finished
                pass
        launch(key)
    else:
        run.data = data
        db.commit()
    return {"ok": True}


@router.post("/runs/{key}/resume")
def resume(key: str, body: ResumeInput, ctx=Depends(context)):
    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    if run.status != "interrupted":
        raise HTTPException(409, "中断した実行だけ再開できます")
    if run.data.get("snapshot", {}).get("mode") != "agent":
        raise HTTPException(409, "再開は長作業モードでのみ利用できます")
    unknown = list(db.scalars(select(ToolCall).where(ToolCall.run_id == key, ToolCall.status == "executing")))
    actions = {item.get("call_id"): item for item in (body.unknown_actions or []) if isinstance(item, dict)}
    if unknown and not body.acknowledge_unknown_result and not actions:
        raise HTTPException(409, "結果不明の操作があります。外部状態を確認してください")
    for call in unknown:
        action = actions.get(call.id, {})
        decision = action.get("decision") if isinstance(action, dict) else None
        if decision == "retry":
            # Leave the call in executing; the resumed drive loop will pick it up.
            continue
        if decision == "mark_unknown":
            message_text = "Operation result was marked unknown by the user; treat as inconclusive."
        else:
            message_text = "Operation result unknown after restart; do not repeat without confirmation."
        call.status = "completed"
        call.data = {
            **call.data,
            "result": {"error": message_text, "type": "unknown_result", "code": "unknown_result"},
        }
        run.data = {
            **run.data,
            "history": [
                *run.data["history"],
                {
                    "role": "tool",
                    "call_id": call.data["provider_call_id"],
                    "name": call.data["name"],
                    "content": message_text,
                },
            ],
        }
    # Optional: restore state from a specific checkpoint.
    if body.from_checkpoint:
        from mix_agent.runs import checkpoints as run_checkpoints

        checkpoint = run_checkpoints.get(db, key, body.from_checkpoint)
        if not checkpoint:
            raise HTTPException(404, "指定したチェックポイントが見つかりません")
        run_checkpoints.apply_to_run(run, checkpoint)
    transition_run(db, run, "queued")
    snapshot = run.data["snapshot"]
    for field in ("max_seconds", "max_steps", "max_tool_calls"):
        value = getattr(body, field)
        if value is not None:
            snapshot[field] = value
    # Resume re-resolves budgets for the (possibly new) model window and
    # validates pinned context refs; missing files degrade to summaries.
    missing_refs: list = []
    for message in run.data.get("history") or []:
        if not isinstance(message, dict):
            continue
        for ref_id in message.get("image_refs") or []:
            row = db.get(Artifact, ref_id)
            if not row or row.owner_id != s.owner_id or not (config.ARTIFACTS / ref_id).exists():
                missing_refs.append(ref_id)
    for ref_id in run.data.get("tool_refs") or []:
        row = db.get(Artifact, ref_id)
        if not row or row.owner_id != s.owner_id or not (config.ARTIFACTS / ref_id).exists():
            missing_refs.append(ref_id)
    run.data = {
        **run.data,
        "snapshot": snapshot,
        "trigger_type": "resume",
        "missing_image_refs": missing_refs,
    }
    # New explicit continuation grants a new time window, not extra tool/step budget.
    run.created_at = now()
    try:
        db.commit()
    except IntegrityError:
        db.rollback()
        raise HTTPException(409, "別の実行が進行中です")
    launch(key)
    return {"ok": True}


@router.post("/runs/{key}/budget-extension")
def decide_budget_extension(key: str, body: BudgetExtensionDecisionInput, ctx=Depends(context)):
    from mix_agent.runs.engine import emit

    db, s = ctx
    run = own(db, Run, key, s.owner_id)
    if run.status != "budget_extension_pending":
        raise HTTPException(409, "予算延長要求中の実行ではありません")
    if body.grant:
        snapshot = run.data.get("snapshot") or {}
        for field in ("max_seconds", "max_steps", "max_tool_calls"):
            value = getattr(body, field)
            if value is not None:
                snapshot[field] = value
        # Bump the effective limits by 50% of the original ceiling when the
        # caller did not specify overrides so a single extension always
        # meaningfully extends the budget.
        policy = snapshot.get("policy") or {}
        max_ext = int(policy.get("max_budget_extensions", 0))
        used = int(run.data.get("budget_extensions_used", 0))
        for field, fallback in (("max_seconds", 5400), ("max_steps", 300), ("max_tool_calls", 750)):
            if snapshot.get(field) is None:
                snapshot[field] = fallback
        if body.max_seconds is None:
            snapshot["max_seconds"] = int(snapshot["max_seconds"] + max(900, snapshot["max_seconds"] // 2))
        if body.max_steps is None:
            snapshot["max_steps"] = int(snapshot["max_steps"] + max(20, snapshot["max_steps"] // 5))
        if body.max_tool_calls is None:
            snapshot["max_tool_calls"] = int(
                snapshot["max_tool_calls"] + max(50, snapshot["max_tool_calls"] // 5)
            )
        run.data = {
            **run.data,
            "snapshot": snapshot,
            "budget_extension_request": None,
        }

        emit(
            db,
            run.id,
            "budget_extension_resolved",
            {
                "grant": True,
                "max_seconds": snapshot["max_seconds"],
                "max_steps": snapshot["max_steps"],
                "max_tool_calls": snapshot["max_tool_calls"],
                "extensions_used": used,
                "extensions_max": max_ext,
            },
        )
        transition_run(db, run, "running", expected_from="budget_extension_pending")
        db.commit()
        launch(run.id)
        return {
            "ok": True,
            "granted": True,
            "snapshot": {
                "max_seconds": snapshot["max_seconds"],
                "max_steps": snapshot["max_steps"],
                "max_tool_calls": snapshot["max_tool_calls"],
            },
        }
    # Denied: stop the run cleanly so the user can inspect the outcome.
    from mix_agent.runs.engine import finish

    run.data = {**run.data, "budget_extension_request": None}
    emit(db, run.id, "budget_extension_resolved", {"grant": False})
    finish(db, run, "completed", "予算の延長が許可されなかったため、Runは取得済みの結果で終了しました。")
    return {"ok": True, "granted": False}


@router.post("/approvals/{key}/decision")
def decide(key: str, body: DecisionInput, ctx=Depends(context)):
    db, s = ctx
    approval = own(db, Approval, key, s.owner_id)
    if approval.status != "pending":
        if approval.status == body.decision:
            return {"ok": True}
        raise HTTPException(409, "承認は決定済みです")
    run = own(db, Run, approval.run_id, s.owner_id)
    if run.data.get("browser_manual_active"):
        raise HTTPException(409, "手動操作を終了してから承認してください")
    if run.status != "waiting_approval" or approval.expires.replace(tzinfo=UTC) < now():
        raise HTTPException(409, "この承認は有効ではありません")
    call = db.get(ToolCall, approval.tool_call_id)
    tool = registry(db, s.owner_id).get(call.data["tool_id"])
    if not tool or fingerprint(tool) != approval.data["tool_version"]:
        raise HTTPException(409, "Tool定義が変更されています")
    approval.status = body.decision
    if body.decision == "always":
        db.add(
            Permission(
                owner_id=s.owner_id,
                data={
                    "agent_id": run.data["snapshot"].get("agent_id", ""),
                    "tool_id": tool["id"],
                    "permission": "allow",
                    "scope": approval.data["scope"],
                    "tool_version": fingerprint(tool),
                },
            )
        )
    audit(db, s.owner_id, "approval." + body.decision, key)
    db.commit()
    launch(run.id)
    return {"ok": True}
