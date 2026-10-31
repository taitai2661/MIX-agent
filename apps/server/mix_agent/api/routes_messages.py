"""Message send/edit, feedback and per-model verification routes."""

import asyncio
import hashlib
import re

from fastapi import APIRouter, Depends, HTTPException, Request
from sqlalchemy import select
from starlette.concurrency import run_in_threadpool

from mix_agent import config
from mix_agent.api.common import audit, context, own, public
from mix_agent.api.message_context import (
    _build_history_payload,
    _handle_explicit_memory_requests,
    _model_window_info as _model_window_info,  # re-exported for api.routes
)
from mix_agent.api.message_runs import persist_message_run, _resolve_tool_ids
from mix_agent.api.schemas import FeedbackInput, MessageInput, SendMessageView
from mix_agent.db.models import (
    Agent,
    Artifact,
    Conversation,
    Feedback,
    Message,
    Model,
    Provider,
    Run,
    Settings,
)
from mix_agent.providers import capability_probe
from mix_agent.providers.probing import (
    capability_targets,
    probe_auto_candidates,
    tool_capability,
    verify_model_capabilities,
    verify_tool_capability,
)
from mix_agent.providers.reasoning import resolve_reasoning
from mix_agent.routing import effective_capabilities, select_auto_model
from mix_agent.runs.engine import launch
from mix_agent.storage.cleanup import _doomed_messages

router = APIRouter(prefix="/api/v1")


async def _create_message_run(
    db,
    s,
    key,
    conversation,
    body,
    request_key,
    request_hash,
    truncate_from_message_id=None,
    keep_artifact_ids=(),
):
    """Resolve model/provider/tools and persist a new Run + user turn.

    Shared by `send_message` and `edit_message`. With `truncate_from_message_id`
    the replaced turn and everything after it are removed in the same final
    commit as the new Run, so a failure while building the Run cannot leave the
    conversation truncated with no replacement. The tail is excluded from the
    prompt history before it is deleted, so the regenerated Run never sees the
    turns it is replacing.
    """
    settings = own(db, Settings, "settings", s.owner_id)
    temporary_mode = body.temporary_mode
    saved_selection = conversation.data.get("selection", {})
    if not isinstance(saved_selection, dict):
        saved_selection = {}
    # Explicit input wins; otherwise a conversation keeps its assistant before
    # falling back to the app default.  Auto is the safe new-chat default.
    requested_model_id = (
        body.model_id
        if body.model_id
        else saved_selection.get("model_id") or settings.data.get("default_model_id") or "auto"
    )
    requested_agent_id = (
        body.agent_id if "agent_id" in body.model_fields_set else saved_selection.get("agent_id") or ""
    )
    mode = body.mode if "mode" in body.model_fields_set else saved_selection.get("mode", "chat")
    if mode not in ("chat", "thinking", "agent"):
        mode = "chat"
    if body.research_mode and mode != "agent":
        raise HTTPException(422, "調査モードはAgentモードで実行してください")
    agent = own(db, Agent, requested_agent_id, s.owner_id).data if requested_agent_id else {}
    _handle_explicit_memory_requests(db, s.owner_id, body.content, temporary_mode)
    artifacts = [own(db, Artifact, artifact_id, s.owner_id) for artifact_id in body.artifact_ids]
    artifact_mimes = [artifact.data["mime"] for artifact in artifacts]
    tool_ids = _resolve_tool_ids(agent, requested_agent_id, mode, temporary_mode, body, settings)
    if body.research_mode and not any(t in tool_ids for t in ("web_search", "web_fetch", "browser_open")):
        raise HTTPException(422, "調査モードにはWeb検索またはBrowser Toolが必要です")
    artifact_requested = bool(
        re.search(
            r"(?:html|css|javascript|js|json|markdown|ファイル|ダウンロード|成果物|コードを保存|単一html)",
            body.content,
            re.IGNORECASE,
        )
    )
    tools_required = bool(tool_ids) and (mode == "agent" or artifact_requested)
    # Turns an edit is replacing are hidden from the prompt now, before they are
    # deleted with the new Run, so history and auto-routing see only the prefix.
    doomed_ids = (
        {message.id for message in _doomed_messages(db, key, truncate_from_message_id)}
        if truncate_from_message_id
        else set()
    )
    prior_content = (
        []
        if temporary_mode
        else [
            message.data.get("content", "")
            for message in db.scalars(
                select(Message).where(Message.conversation_id == key).order_by(Message.created_at, Message.id)
            )
            if message.id not in doomed_ids
        ]
    )
    auto_selection = None
    if requested_model_id == "auto":
        allowed_ids = settings.data.get("auto_model_ids", [])
        if not allowed_ids:
            raise HTTPException(422, "Autoで使用可能なモデルを設定してください")
        task_content = body.content + ("\n" + agent.get("system_prompt", "") if mode == "agent" else "")
        # Confirm unknown Tool Calling support before Auto filters candidates, so
        # a capable model is not excluded just because it was never checked.
        if tools_required:
            try:
                await asyncio.wait_for(
                    probe_auto_candidates(db, s.owner_id, allowed_ids, ("tools",)),
                    timeout=config.PROBE_TICK_TIMEOUT_SECONDS,
                )
            except Exception:  # noqa: BLE001 - a slow probe must not block the message
                db.rollback()
        model, auto_selection = await run_in_threadpool(
            select_auto_model,
            db,
            s.owner_id,
            allowed_ids,
            task_content,
            mode,
            artifact_mimes,
            tools_required,
            [*prior_content, body.content, agent.get("system_prompt", "")],
            agent.get("model_settings", {}).get("max_output_tokens", 4096),
            request_key,
            sum(a.data.get("size", 0) for a in artifacts),
            priority=settings.data.get("auto_priority", "balanced"),
        )
        if not model:
            raise HTTPException(422, auto_selection["reason"])
    else:
        model = own(db, Model, requested_model_id, s.owner_id)
    provider = own(db, Provider, model.data["provider_id"], s.owner_id)
    caps = effective_capabilities(model.data)
    try:
        reasoning = resolve_reasoning(
            provider.data["kind"], model.data["model_id"], caps, mode, agent.get("model_settings", {})
        )
    except ValueError as exc:
        raise HTTPException(422, str(exc))
    tools_capability = tool_capability(model.data)
    # Run a one-call Tool Calling probe for every interactive mode (chat,
    # thinking, agent) when the capability is unknown and the stored verdict is
    # stale or absent. capability_probe.probe_needed also re-checks outdated
    # verdicts, so this catches both fresh models and older probe_versions.
    # Manual overrides already short-circuit probe_needed, so this never
    # contradicts an explicit user setting.
    if (
        tool_ids
        and tools_capability is None
        and capability_probe.probe_needed(model.data, "tools", provider.data.get("kind"))
    ):
        tools_capability = await verify_tool_capability(db, s.owner_id, model, provider, "automatic")
        tools_capability = {"supported": True, "unsupported": False}.get(tools_capability)
    if tool_ids and tools_capability is not True:
        # A failed (or inconclusive) probe never blocks a run: tools are dropped
        # silently and the scheduler tick retries on the next due check. Plain
        # conversation stays available on models without Tool Calling.
        tool_ids = []
    snapshot, history, _, memory_trace_ids, _ = await _build_history_payload(
        db,
        s,
        conversation,
        body,
        settings,
        agent,
        artifacts,
        mode,
        requested_model_id,
        requested_agent_id,
        model,
        provider,
        caps,
        reasoning,
        tool_ids,
        temporary_mode,
        exclude_message_ids=doomed_ids,
    )
    auto_routing = (
        {
            "allowed_ids": allowed_ids,
            "content": body.content + ("\n" + agent.get("system_prompt", "") if mode == "agent" else ""),
            "mode": mode,
            "artifact_mimes": artifact_mimes,
            "tools_required": tools_required,
            "context_parts": [*prior_content, body.content, agent.get("system_prompt", "")],
            "reserved_output_tokens": agent.get("model_settings", {}).get("max_output_tokens", 4096),
            "attachment_bytes": sum(a.data.get("size", 0) for a in artifacts),
            "dynamic_switching": bool(settings.data.get("auto_dynamic_switching", True)),
            "priority": settings.data.get("auto_priority", "balanced"),
        }
        if requested_model_id == "auto"
        else None
    )
    # Delete the replaced tail in the same transaction as the Run insert:
    # `persist_message_run` commits once, so either both happen or neither does.
    # The orphaned files are unlinked only after that commit succeeds — a
    # rollback must restore the rows without the bytes having already vanished.
    run = await run_in_threadpool(
        persist_message_run,
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
    )
    return run


@router.post("/conversations/{key}/messages", response_model=SendMessageView)
async def send_message(key: str, body: MessageInput, request: Request, ctx=Depends(context)):
    db, s = ctx
    conversation = own(db, Conversation, key, s.owner_id)
    if conversation.data.get("deleted_at") or conversation.data.get("archived_at"):
        raise HTTPException(409, "アーカイブまたはごみ箱から復元してから続けてください")
    request_key = request.headers.get("idempotency-key", "")
    request_hash = hashlib.sha256(body.model_dump_json().encode()).hexdigest()
    if not 8 <= len(request_key) <= 100:
        raise HTTPException(422, "Idempotency-Keyが必要です")
    existing = db.scalar(select(Run).where(Run.request_key == request_key))
    if existing:
        if existing.owner_id != s.owner_id or existing.conversation_id != key:
            raise HTTPException(409)
        if existing.data.get("request_hash") != request_hash:
            raise HTTPException(409, "同じIdempotency-Keyで異なる入力は送信できません")
        return {"run_id": existing.id}
    run = await _create_message_run(db, s, key, conversation, body, request_key, request_hash)
    launch(run.id)
    return {"run_id": run.id}


@router.post("/conversations/{key}/messages/{message_id}/edit", response_model=SendMessageView)
async def edit_message(key: str, message_id: str, body: MessageInput, request: Request, ctx=Depends(context)):
    """Replace a user turn and regenerate from it.

    The edited turn and every turn after it are removed together with the new
    Run, so the regenerated Run is built from the surviving prefix alone and a
    failure while building it leaves the original conversation intact. Keeping
    later replies would leave them answering wording the user never sent.
    """
    db, s = ctx
    conversation = own(db, Conversation, key, s.owner_id)
    if conversation.data.get("deleted_at") or conversation.data.get("archived_at"):
        raise HTTPException(409, "アーカイブまたはごみ箱から復元してから続けてください")
    request_key = request.headers.get("idempotency-key", "")
    # Hash the request as received, before attachments are carried over from the
    # replaced turn, so retrying the same edit is recognised as the same request.
    # The target turn is part of the hash so reusing a key across different
    # messages is rejected rather than mistaken for a retry of this edit.
    request_hash = hashlib.sha256(
        b"edit:" + message_id.encode() + b":" + body.model_dump_json().encode()
    ).hexdigest()
    if not 8 <= len(request_key) <= 100:
        raise HTTPException(422, "Idempotency-Keyが必要です")
    existing = db.scalar(select(Run).where(Run.request_key == request_key))
    if existing:
        if existing.owner_id != s.owner_id or existing.conversation_id != key:
            raise HTTPException(409)
        if existing.data.get("request_hash") != request_hash:
            raise HTTPException(409, "同じIdempotency-Keyで異なる入力は送信できません")
        # A retry of an edit that already succeeded: the target turn is gone, so
        # return the Run the first attempt created instead of deleting again.
        return {"run_id": existing.id}
    message = own(db, Message, message_id, s.owner_id)
    if message.conversation_id != key or message.data.get("role") != "user":
        raise HTTPException(422, "ユーザーのメッセージのみ編集できます")
    if not body.content.strip():
        raise HTTPException(422, "メッセージを入力してください")
    active = db.scalar(
        select(Run).where(
            Run.conversation_id == key,
            Run.status.in_(("queued", "running", "waiting_approval", "paused", "budget_extension_pending")),
        )
    )
    if active:
        raise HTTPException(409, "実行中はメッセージを編集できません")
    # The editor only changes the text; keep the images/files the turn carried
    # so editing an image question does not silently drop the image.
    if not body.artifact_ids and message.data.get("artifact_ids"):
        body.artifact_ids = list(message.data["artifact_ids"])
    run = await _create_message_run(
        db,
        s,
        key,
        conversation,
        body,
        request_key,
        request_hash,
        truncate_from_message_id=message_id,
        keep_artifact_ids=list(body.artifact_ids),
    )
    launch(run.id)
    return {"run_id": run.id}


@router.put("/messages/{key}/feedback")
def set_message_feedback(key: str, body: FeedbackInput, ctx=Depends(context)):
    db, s = ctx
    message = own(db, Message, key, s.owner_id)
    if message.data.get("role") != "assistant" or not message.data.get("auto_selection"):
        raise HTTPException(422, "Autoで選択された回答のみ評価できます")
    old = db.scalar(select(Feedback).where(Feedback.message_id == key))
    if body.value is None:
        if old:
            db.delete(old)
    elif old:
        old.data = {**old.data, "value": body.value}
    else:
        selection = message.data["auto_selection"]
        db.add(
            Feedback(
                owner_id=s.owner_id,
                message_id=key,
                data={
                    "model_id": selection["model_record_id"],
                    "profile": selection["profile"],
                    "value": body.value,
                },
            )
        )
    audit(db, s.owner_id, "message.feedback", key)
    db.commit()
    return {"value": body.value}


@router.post("/models/{key}/verify-tools")
def verify_model_tools(key: str, ctx=Depends(context)):
    db, s = ctx
    model = own(db, Model, key, s.owner_id)
    provider = own(db, Provider, model.data["provider_id"], s.owner_id)
    # A manual re-check always re-confirms Tool Calling and additionally covers
    # any other capability that is still unknown or stale.
    targets = list(dict.fromkeys(["tools", *capability_targets(model.data, provider.data.get("kind"))]))
    # `def` keeps the synchronous SQLAlchemy work on the threadpool. The probe
    # itself is async, so it gets a loop scoped to this worker thread rather
    # than blocking the server loop with synchronous DB calls.
    asyncio.run(verify_model_capabilities(db, s.owner_id, model, provider, "manual", targets))
    audit(db, s.owner_id, "model.verify_tools", key)
    db.commit()
    return public(model)
