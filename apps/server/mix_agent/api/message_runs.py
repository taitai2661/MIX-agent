"""Run/message persistence helpers for a new message."""

from fastapi import HTTPException
from sqlalchemy.exc import IntegrityError

from mix_agent.api.common import conversation_title as _conversation_title
from mix_agent.api.routes_conversations import effective_tool_ids
from mix_agent.context import task_state as context_task_state
from mix_agent.db.models import Message, Run, now
from mix_agent.storage.cleanup import _delete_message_tail
from mix_agent.tools.registry import BUILTINS


def _resolve_tool_ids(agent, requested_agent_id, mode, temporary_mode, body, settings):
    """Resolve the effective tool id list for a run given mode, agent and settings."""
    tool_ids = agent.get("tool_ids", [t["id"] for t in BUILTINS])
    if mode == "agent" and not temporary_mode and not requested_agent_id and agent.get("auto_learn", True):
        tool_ids = list(
            dict.fromkeys(
                [
                    *tool_ids,
                    "memory_search",
                    "memory_add",
                    "memory_update",
                    "skill_search",
                    "skill_add",
                    "skill_update",
                ]
            )
        )
    tool_ids = (
        effective_tool_ids(settings, agent, tool_ids) if (not temporary_mode or body.allow_tools) else []
    )
    return [
        tool_id for tool_id in tool_ids if not temporary_mode or not tool_id.startswith(("memory_", "skill_"))
    ]


def _build_run_record(
    db,
    s,
    conversation,
    key,
    body,
    request_key,
    request_hash,
    snapshot,
    history,
    artifacts,
    auto_selection,
    auto_routing,
    memory_trace_ids,
    temporary_mode,
    prior_content,
):
    """Create and return the persistent Run + Message records for a new message."""
    run = Run(
        owner_id=s.owner_id,
        conversation_id=key,
        request_key=request_key,
        data={
            "snapshot": snapshot,
            "history": history,
            "steps": 0,
            "tool_count": 0,
            "checkpoint": (
                {"status": "started", "steps": 0, "tool_count": 0}
                if snapshot.get("mode") == "agent"
                else None
            ),
            "request_hash": request_hash,
            "auto_selection": auto_selection,
            "auto_routing": auto_routing,
            "auto_model_history": (
                [
                    {
                        "step": 1,
                        "to_model_record_id": snapshot["model_record_id"],
                        "model_id": snapshot["model_id"],
                    }
                ]
                if auto_selection
                else []
            ),
            "requested_model_id": snapshot.get("requested_model_id"),
            "memory_trace_ids": memory_trace_ids,
            "temporary_mode": temporary_mode,
            "allow_tools": body.allow_tools,
            "temporary_artifact_ids": list(body.artifact_ids) if temporary_mode else [],
            "temporary_conversation": temporary_mode
            and conversation.data.get("title") == "新しいチャット"
            and not prior_content,
            # Phase 1 Context Engine state (progressive summary + task state + trace).
            "task_state": (snapshot.get("_context_bootstrap") or {}).get("task_state")
            or context_task_state.blank(),
            "summary": {"text": "", "covered_count": 0, "updated_at": None},
            "context_trace": snapshot.get("context_trace_bootstrap"),
            "context_version": 1,
            "trigger_type": "interactive",
            "image_refs": list(body.artifact_ids),
            "tool_refs": [],
        },
    )
    db.add(run)
    message = Message(
        owner_id=s.owner_id,
        conversation_id=key,
        data={
            "role": "user",
            "content": body.content,
            "run_id": run.id,
            "artifact_ids": body.artifact_ids,
            "artifacts": [{"artifact_id": artifact.id, **artifact.data} for artifact in artifacts],
        },
    )
    db.add(message)
    db.flush()
    run.data = {**run.data, "input_message_id": message.id}
    if not temporary_mode and conversation.data.get("title") == "新しいチャット":
        conversation.data = {
            **conversation.data,
            "title": _conversation_title(body.content or "添付について"),
        }
    if not temporary_mode:
        conversation.data = {
            **conversation.data,
            "selection": {
                "model_id": snapshot.get("requested_model_id"),
                "agent_id": snapshot.get("agent_id", ""),
                "mode": snapshot.get("mode", "chat"),
                "research_mode": body.research_mode,
            },
            "last_message_at": now().isoformat(),
        }
    try:
        db.commit()
    except IntegrityError:
        db.rollback()
        raise HTTPException(409, "この会話は実行中です")
    return run


def persist_message_run(
    db,
    s,
    conversation,
    key,
    body,
    request_key,
    request_hash,
    snapshot,
    history,
    artifacts,
    auto_selection,
    auto_routing,
    memory_trace_ids,
    temporary_mode,
    prior_content,
    truncate_from_message_id,
    keep_artifact_ids,
):
    """Delete a replaced tail and commit the new Run in one blocking unit.

    Called through ``run_in_threadpool`` from the async route: the commit and
    the orphan cleanup are the slowest synchronous parts of sending a message,
    and with one uvicorn worker they would otherwise stall every other request.
    The delete and the insert share the transaction, so a failure leaves both
    untouched; files are unlinked only after the commit succeeds.
    """
    orphaned_paths = []
    if truncate_from_message_id:
        orphaned_paths = _delete_message_tail(
            db,
            conversation,
            truncate_from_message_id,
            keep_artifact_ids=keep_artifact_ids,
        )
    run = _build_run_record(
        db,
        s,
        conversation,
        key,
        body,
        request_key,
        request_hash,
        snapshot,
        history,
        artifacts,
        auto_selection,
        auto_routing,
        memory_trace_ids,
        temporary_mode,
        prior_content,
    )
    for path in orphaned_paths:
        path.unlink(missing_ok=True)
    return run
