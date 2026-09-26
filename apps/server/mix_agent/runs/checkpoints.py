"""Per-step Run checkpoints for selective resume.

The engine writes a checkpoint on a fixed cadence (default every 10 steps) and
on natural boundaries (status changes, after a write tool completes, before a
risk-managed tool starts).  Each checkpoint captures the structured state
needed to resume a Run from that exact step without losing context.

The on-Run ``run.data.checkpoint`` field remains the final/working summary for
the simple UI; this module is the durable history used for selective resume.
"""

from __future__ import annotations

import json
from copy import deepcopy
from typing import Any

from sqlalchemy import select

from mix_agent.db.models import Run, RunCheckpoint
from mix_agent.runs import state as run_state

# Default cadence.  Stored on the snapshot so future Run-spec changes do not
# silently tighten or relax the cadence for in-flight Runs.
DEFAULT_CHECKPOINT_EVERY_STEPS = 10
# Hard cap on the number of checkpoints kept per Run to bound storage growth.
DEFAULT_KEEP_RECENT = 20

_TRUNCATE_HISTORY_FROM = -8
_KEEP_RECENT_TOOL_CALLS = 12


def checkpoint_every_steps(snapshot: dict | None) -> int:
    """Return the configured step interval for periodic checkpoints."""
    if not isinstance(snapshot, dict):
        return DEFAULT_CHECKPOINT_EVERY_STEPS
    raw = (snapshot.get("policy") or {}).get("checkpoint_every_steps")
    if isinstance(raw, int) and not isinstance(raw, bool) and 1 <= raw <= 100:
        return raw
    return DEFAULT_CHECKPOINT_EVERY_STEPS


def keep_recent(snapshot: dict | None) -> int:
    """Return the per-Run retention cap for checkpoints."""
    if not isinstance(snapshot, dict):
        return DEFAULT_KEEP_RECENT
    raw = (snapshot.get("policy") or {}).get("checkpoint_keep_recent")
    if isinstance(raw, int) and not isinstance(raw, bool) and 5 <= raw <= 100:
        return raw
    return DEFAULT_KEEP_RECENT


def _truncate_history_for_snapshot(history: list[dict] | None) -> list[dict]:
    """Keep only the head + tail of history to bound checkpoint storage.

    The head carries the system block (which holds frozen mode prompt, task
    state, summary, and references).  The tail keeps the most recent turns so
    a resume sees the same context as before the checkpoint.
    """
    if not history:
        return []
    if len(history) <= 1:
        return list(history)
    head, tail = history[:1], history[-_TRUNCATE_HISTORY_FROM:]
    return [*head, *tail]


def _recent_tool_calls(db, run_id: str) -> list[dict]:
    """Capture the most recent tool calls in provider-call order."""
    from mix_agent.db.models import ToolCall

    rows = list(
        db.scalars(
            select(ToolCall)
            .where(ToolCall.run_id == run_id)
            .order_by(ToolCall.created_at.desc())
            .limit(_KEEP_RECENT_TOOL_CALLS)
        )
    )
    rows.reverse()
    snapshot = []
    for row in rows:
        snapshot.append(
            {
                "id": row.id,
                "status": row.status,
                "data": deepcopy(row.data or {}),
            }
        )
    return snapshot


def capture_snapshot(db, run: Run, *, trigger: str) -> dict:
    """Build a persistable checkpoint payload from the current Run state."""
    data = run.data or {}
    snapshot = data.get("snapshot") or {}
    return {
        "trigger": trigger,
        "step": int(data.get("steps", 0)),
        "tool_count": int(data.get("tool_count", 0)),
        "history": _truncate_history_for_snapshot(data.get("history") or []),
        "task_state": deepcopy(data.get("task_state") or {}),
        "agent_plan": deepcopy(data.get("agent_plan") or {}),
        "summary": deepcopy(data.get("summary") or {}),
        "context_trace": deepcopy(data.get("context_trace") or {}),
        "missing_image_refs": list(data.get("missing_image_refs") or []),
        "tool_refs": list(data.get("tool_refs") or []),
        "recent_tool_calls": _recent_tool_calls(db, run.id),
        "browser_frames": deepcopy(data.get("browser_frames") or []),
        "mode": snapshot.get("mode"),
        "policy": deepcopy(snapshot.get("policy") or {}),
        "remaining": {
            "max_seconds": snapshot.get("max_seconds"),
            "max_steps": snapshot.get("max_steps"),
            "max_tool_calls": snapshot.get("max_tool_calls"),
        },
        "created_at_kind": "interval" if trigger == "interval" else trigger,
    }


def save(db, run: Run, *, trigger: str = "interval") -> RunCheckpoint:
    """Persist a new checkpoint and trim older ones beyond the retention cap."""
    payload = capture_snapshot(db, run, trigger=trigger)
    checkpoint = RunCheckpoint(
        owner_id=run.owner_id,
        run_id=run.id,
        step=payload["step"],
        tool_count=payload["tool_count"],
        trigger=trigger,
        data=payload,
    )
    db.add(checkpoint)
    db.flush()
    _trim(db, run.id, keep_recent(run.data.get("snapshot")))
    return checkpoint


def list_for_run(db, run_id: str) -> list[dict]:
    """Return a public view of all checkpoints for a Run, newest first."""
    rows = list(
        db.scalars(
            select(RunCheckpoint)
            .where(RunCheckpoint.run_id == run_id)
            .order_by(RunCheckpoint.created_at.desc())
        )
    )
    return [
        {
            "id": row.id,
            "step": row.step,
            "tool_count": row.tool_count,
            "trigger": row.trigger,
            "created_at": row.created_at.isoformat() if row.created_at else None,
        }
        for row in rows
    ]


def get(db, run_id: str, checkpoint_id: str) -> RunCheckpoint | None:
    return db.scalar(
        select(RunCheckpoint)
        .where(RunCheckpoint.run_id == run_id, RunCheckpoint.id == checkpoint_id)
    )


def apply_to_run(run: Run, checkpoint: RunCheckpoint) -> None:
    """Restore a Run's structured state from a checkpoint payload.

    The durable fields (history, task_state, agent_plan, summary, counters)
    are overwritten so the next drive() step starts from the saved point.
    Callers must persist the change after this returns.
    """
    payload = checkpoint.data or {}
    history = list(payload.get("history") or [])
    merged: dict[str, Any] = {
        **run.data,
        "history": history,
        "task_state": deepcopy(payload.get("task_state") or {}),
        "agent_plan": deepcopy(payload.get("agent_plan") or {}),
        "summary": deepcopy(payload.get("summary") or {}),
        "context_trace": deepcopy(payload.get("context_trace") or {}),
        "missing_image_refs": list(payload.get("missing_image_refs") or []),
        "tool_refs": list(payload.get("tool_refs") or []),
        "browser_frames": deepcopy(payload.get("browser_frames") or []),
        "steps": int(payload.get("step", run.data.get("steps", 0))),
        "tool_count": int(payload.get("tool_count", run.data.get("tool_count", 0))),
        "checkpoint_resumed_from": {
            "id": checkpoint.id,
            "step": checkpoint.step,
            "created_at": checkpoint.created_at.isoformat() if checkpoint.created_at else None,
        },
    }
    run.data = merged
    # Carry remaining budget overrides back into the snapshot so drive() sees
    # the same ceiling that was active at the checkpoint boundary.
    snapshot = dict(run.data.get("snapshot") or {})
    remaining = payload.get("remaining") or {}
    for key in ("max_seconds", "max_steps", "max_tool_calls"):
        value = remaining.get(key)
        if isinstance(value, int) and not isinstance(value, bool):
            snapshot[key] = value
    run.data["snapshot"] = snapshot


def _trim(db, run_id: str, keep: int) -> None:
    """Delete the oldest checkpoints beyond the retention cap."""
    from sqlalchemy import delete

    rows = list(
        db.scalars(
            select(RunCheckpoint)
            .where(RunCheckpoint.run_id == run_id)
            .order_by(RunCheckpoint.created_at.desc())
        )
    )
    if len(rows) <= keep:
        return
    for row in rows[keep:]:
        db.execute(delete(RunCheckpoint).where(RunCheckpoint.id == row.id))


def should_checkpoint(snapshot: dict, step: int) -> bool:
    """Return whether a periodic checkpoint should be saved at this step."""
    if step <= 0:
        return False
    if step == 1:
        return True
    return step % checkpoint_every_steps(snapshot) == 0


def is_recoverable(run: Run) -> bool:
    """Whether the Run can be resumed (mirrors the API check)."""
    return run.status == "interrupted" and (run.data.get("snapshot") or {}).get("mode") == "agent"


def status_event_payload(run: Run, reason: str | None = None) -> dict:
    """Build a status event payload for cross-module consumers."""
    return {
        "status": run.status,
        "from_status": run.data.get("status_from"),
        "reason": reason,
        "is_terminal": run.status in run_state.TERMINAL_STATES,
    }


def safe_json_dumps(value: Any) -> str:
    """Render a checkpoint payload for logging without leaking secrets."""
    return json.dumps(value, ensure_ascii=False, default=str)[:1000]
