"""Cleanup helpers for purging conversations, runs, and expired data.

These functions were extracted from mix_agent.api.routes to keep the route
module manageable.  They are re-exported from routes.py for backward compat.
"""
from datetime import timedelta

from sqlalchemy import select

from mix_agent import config
from mix_agent.db.models import (
    Approval,
    Artifact,
    Conversation,
    Event,
    Feedback,
    Message,
    Run,
    RunCheckpoint,
    ToolCall,
    now,
)


def _collect_used_artifact_ids(db, owner_id: str) -> set:
    """Return all artifact IDs currently referenced by any message or run for owner."""
    used: set = set()
    for message in db.scalars(select(Message).where(Message.owner_id == owner_id)):
        used.update(_conversation_artifact_ids([message]))
    for run in db.scalars(select(Run).where(Run.owner_id == owner_id)):
        used.update(_run_context_artifact_ids(run))
        used.update(
            item.get("artifact_id")
            for item in run.data.get("browser_frames", [])
            if item.get("artifact_id")
        )
    return used


def _run_context_artifact_ids(run) -> set:
    """Collect context-internal artifact refs pinned to a run.

    Covers image_refs / tool_refs / task_state artifacts / summary refs so
    purge paths never orphan context files and never miss run-pinned files.
    """
    ids: set = set()
    try:
        data = run.data or {}
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        return ids
    for key in ("tool_refs", "image_refs"):
        for value in data.get(key) or []:
            if isinstance(value, str) and value:
                ids.add(value)
    task_state = data.get("task_state") or {}
    for item in task_state.get("artifacts") or []:
        if isinstance(item, str) and item:
            ids.add(item)
        elif isinstance(item, dict) and item.get("artifact_id"):
            ids.add(item["artifact_id"])
    summary = data.get("summary") or {}
    for value in summary.get("artifact_refs") or []:
        if isinstance(value, str) and value:
            ids.add(value)
    for message in data.get("history") or []:
        if not isinstance(message, dict):
            continue
        if message.get("tool_ref"):
            ids.add(message["tool_ref"])
        for ref in message.get("image_refs") or []:
            ids.add(ref)
    for item in data.get("artifacts") or []:
        if isinstance(item, dict) and item.get("artifact_id"):
            ids.add(item["artifact_id"])
    return {i for i in ids if isinstance(i, str)}


def _conversation_artifact_ids(messages, runs=None):
    ids = set()
    for message in messages:
        ids.update(message.data.get("artifact_ids", []))
        ids.update(item.get("artifact_id") for item in message.data.get("artifacts", []) if item.get("artifact_id"))
    for run in runs or []:
        ids.update(_run_context_artifact_ids(run))
        ids.update(item.get("artifact_id") for item in run.data.get("browser_frames", []) if item.get("artifact_id"))
    return ids


def _doomed_messages(db, conversation_id, from_message_id):
    """Messages from `from_message_id` (inclusive) through the end of the thread.

    Ordered by `(created_at, id)`: `created_at` alone is not unique, and an
    unstable tie order would change which turns count as "after" the edit.
    """
    rows = list(
        db.scalars(
            select(Message)
            .where(Message.conversation_id == conversation_id)
            .order_by(Message.created_at, Message.id)
        )
    )
    cut = next((index for index, message in enumerate(rows) if message.id == from_message_id), len(rows))
    return rows[cut:]


def _delete_message_tail(db, conversation, from_message_id, keep_artifact_ids=()):
    """Delete a user turn and every turn after it, together with their Runs.

    Used by edit-and-resend: once the replaced turn and the replies to it are
    gone, a regenerated Run is built from the surviving prefix alone. Shared
    artifacts are retained — only files no surviving message or run references
    are removed — so editing one conversation cannot delete another's uploads.
    `keep_artifact_ids` are attachments the replacement turn reuses; they must
    survive even though the only message referencing them is being deleted.

    Returns the on-disk paths of artifacts that became orphaned. The caller must
    unlink them only after the surrounding transaction commits: deleting files
    here would leave the bytes gone while a rolled-back transaction restored the
    rows that still reference them.
    """
    doomed = _doomed_messages(db, conversation.id, from_message_id)
    if not doomed:
        return []
    orphaned_paths = []
    message_ids = [message.id for message in doomed]
    doomed_id_set = set(message_ids)
    run_ids = {message.data.get("run_id") for message in doomed if message.data.get("run_id")}
    runs = list(db.scalars(select(Run).where(Run.conversation_id == conversation.id)))
    # The replaced turn's own Run is not referenced by any *surviving* message,
    # so it is picked up through `input_message_id` instead of only `run_id`.
    doomed_runs = [
        run for run in runs if run.id in run_ids or run.data.get("input_message_id") in doomed_id_set
    ]
    for run in doomed_runs:
        run_ids.add(run.id)
    artifact_ids = set()
    for message in doomed:
        artifact_ids.update(message.data.get("artifact_ids", []))
        artifact_ids.update(
            item.get("artifact_id") for item in message.data.get("artifacts", []) if item.get("artifact_id")
        )
    for run in doomed_runs:
        artifact_ids.update(_run_context_artifact_ids(run))
        artifact_ids.update(
            item.get("artifact_id") for item in run.data.get("browser_frames", []) if item.get("artifact_id")
        )
    if run_ids:
        for approval in db.scalars(select(Approval).where(Approval.run_id.in_(run_ids))):
            db.delete(approval)
        for call in db.scalars(select(ToolCall).where(ToolCall.run_id.in_(run_ids))):
            db.delete(call)
        for event in db.scalars(select(Event).where(Event.run_id.in_(run_ids))):
            db.delete(event)
        for checkpoint in db.scalars(select(RunCheckpoint).where(RunCheckpoint.run_id.in_(run_ids))):
            db.delete(checkpoint)
        for run in db.scalars(select(Run).where(Run.id.in_(run_ids))):
            db.delete(run)
    if message_ids:
        for feedback in db.scalars(select(Feedback).where(Feedback.message_id.in_(message_ids))):
            db.delete(feedback)
    for message in doomed:
        db.delete(message)
    db.flush()
    if artifact_ids:
        used: set = _collect_used_artifact_ids(db, conversation.owner_id)
        for artifact_id in artifact_ids - used - set(keep_artifact_ids):
            artifact = db.get(Artifact, artifact_id)
            if artifact and artifact.owner_id == conversation.owner_id:
                orphaned_paths.append(config.ARTIFACTS / artifact.id)
                db.delete(artifact)
    return orphaned_paths


def purge_temporary_run(db, run):
    """Remove all MIX-local records and files created for a temporary run."""
    run_id = run.id
    messages = list(db.scalars(select(Message).where(Message.data["run_id"].as_string() == run_id)))
    artifact_ids = set(run.data.get("temporary_artifact_ids", []))
    artifact_ids.update(item.get("artifact_id") for item in run.data.get("artifacts", []) if item.get("artifact_id"))
    artifact_ids.update(_run_context_artifact_ids(run))
    artifact_ids.update(item.get("artifact_id") for item in run.data.get("browser_frames", []) if item.get("artifact_id"))
    for message in messages:
        artifact_ids.update(message.data.get("artifact_ids", []))
        db.delete(message)
    message_ids = [message.id for message in messages]
    for model, column, value in ((Feedback, Feedback.message_id, message_ids), (Approval, Approval.run_id, [run_id]),
                                 (ToolCall, ToolCall.run_id, [run_id]), (Event, Event.run_id, [run_id])):
        if not value:
            continue
        for row in list(db.scalars(select(model).where(column.in_(value)))):
            db.delete(row)
    db.delete(run)
    db.flush()
    for artifact_id in artifact_ids:
        artifact = db.get(Artifact, artifact_id)
        if artifact:
            path = config.ARTIFACTS / artifact_id
            if path.exists():
                path.unlink()
            db.delete(artifact)
    if run.data.get("temporary_conversation"):
        conversation = db.get(Conversation, run.conversation_id)
        if conversation:
            db.delete(conversation)


def purge_conversation(db, conversation):
    """Remove a trashed conversation and its dependent records, retaining shared artifacts."""
    runs = list(db.scalars(select(Run).where(Run.conversation_id == conversation.id)))
    run_ids = [run.id for run in runs]
    messages = list(db.scalars(select(Message).where(Message.conversation_id == conversation.id)))
    artifact_ids = _conversation_artifact_ids(messages, runs)
    if run_ids:
        for approval in db.scalars(select(Approval).where(Approval.run_id.in_(run_ids))): db.delete(approval)
        for call in db.scalars(select(ToolCall).where(ToolCall.run_id.in_(run_ids))): db.delete(call)
        for event in db.scalars(select(Event).where(Event.run_id.in_(run_ids))): db.delete(event)
        for run in runs: db.delete(run)
    for feedback in db.scalars(select(Feedback).where(Feedback.message_id.in_([m.id for m in messages]))): db.delete(feedback)
    for message in messages: db.delete(message)
    db.flush()
    # Compute surviving artifact references once instead of rescanning every
    # message and run for each candidate artifact.
    used = _collect_used_artifact_ids(db, conversation.owner_id)
    for artifact_id in artifact_ids:
        if artifact_id not in used:
            artifact = db.get(Artifact, artifact_id)
            if artifact and artifact.owner_id == conversation.owner_id:
                (config.ARTIFACTS / artifact.id).unlink(missing_ok=True)
                db.delete(artifact)
    db.delete(conversation)


def purge_expired_conversations(db):
    cutoff = now() - timedelta(days=30)
    expired_ids = []
    for conversation in db.scalars(select(Conversation)):
        deleted_at = conversation.data.get("deleted_at")
        if not deleted_at:
            continue
        try:
            from datetime import datetime
            if datetime.fromisoformat(deleted_at) <= cutoff:
                expired_ids.append(conversation.id)
        except ValueError:
            continue
    # Purge in bounded batches so a large cleanup never holds one huge transaction.
    for start in range(0, len(expired_ids), 100):
        batch = expired_ids[start:start + 100]
        for conversation in db.scalars(select(Conversation).where(Conversation.id.in_(batch))):
            purge_conversation(db, conversation)
        db.commit()
