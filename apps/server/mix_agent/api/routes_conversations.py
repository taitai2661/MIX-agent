from datetime import datetime
from typing import Literal

from fastapi import APIRouter, Depends, HTTPException, Query
from fastapi.responses import PlainTextResponse
from sqlalchemy import case, func, or_, select

from mix_agent import todos as conversation_todos
from mix_agent.api.common import (
    audit,
    context,
    own,
    public,
)
from mix_agent.api.schemas import (
    ConversationMessagesView,
    ConversationStateInput,
    RecordView,
    ToolCallHistoryView,
)
from mix_agent.auth.security import (
    read_secret,
)
from mix_agent.db.models import (
    Approval,
    Conversation,
    ConversationFolder,
    Feedback,
    Message,
    Project,
    Provider,
    Run,
    ToolCall,
    now,
)
from mix_agent.memory import embeddings as mem_embeddings
from mix_agent.runs.engine import TASKS
from mix_agent.runs.state import transition_run
from mix_agent.tools.execute import is_user_artifact, runner_request

from mix_agent.storage.cleanup import (
    purge_conversation,
)

router = APIRouter(prefix="/api/v1")


async def _memory_query_vector(db, owner_id: str, query: str, provider=None):
    """Query embedding used to rank memory; ``None`` means lexical ranking.

    Pass the provider the run is actually using so the query lands in the same
    embedding space as the stored vectors. Without one we fall back to the
    owner's most recent provider, and with neither we simply rank lexically —
    never an error, because context build must not fail over a missing vector.
    """
    if provider is None:
        provider = db.scalar(
            select(Provider)
            .where(Provider.owner_id == owner_id)
            .order_by(Provider.created_at.desc())
            .limit(1)
        )
    if provider is None:
        return None
    key = read_secret(db, provider.data.get("secret_id"))
    return await mem_embeddings.query_vector({**provider.data, "id": provider.id}, key, query)


def effective_tool_ids(settings, agent, requested):
    global_settings = settings.data.get("tool_settings", {})
    agent_settings = agent.get("tool_settings", {})
    result = []
    for tool_id in requested:
        values = {**global_settings.get(tool_id, {}), **agent_settings.get(tool_id, {})}
        if values.get("enabled", True) is not False:
            result.append(tool_id)
    if settings.data.get("browser_enabled", True) is False:
        result = [x for x in result if not x.startswith("browser_")]
    if settings.data.get("web_search_enabled", True) is False:
        result = [x for x in result if x not in ("web_search", "web_fetch")]
    return result


@router.get("/conversations", response_model=list[RecordView])
def list_conversations(
    state: Literal["active", "archived", "trash"] = "active",
    folder_id: str | None = None,
    project_id: str | None = None,
    q: str = "",
    limit: int | None = Query(default=None, ge=1, le=500),
    offset: int = Query(default=0, ge=0),
    ctx=Depends(context),
):
    """List conversations using SQL filters instead of loading every row.

    The old implementation loaded every conversation (and, when searching,
    every message for every conversation) into Python.  Filtering and paging
    now happen in PostgreSQL; ``limit`` is optional for compatibility with the
    existing sidebar, which still asks for the full active list.
    """
    db, s = ctx
    if project_id:
        own(db, Project, project_id, s.owner_id)
    deleted = Conversation.data["deleted_at"].astext
    archived = Conversation.data["archived_at"].astext
    statement = select(Conversation).where(Conversation.owner_id == s.owner_id)
    if state == "trash":
        statement = statement.where(deleted.is_not(None))
    elif state == "archived":
        statement = statement.where(deleted.is_(None), archived.is_not(None))
    else:
        statement = statement.where(
            deleted.is_(None),
            archived.is_(None),
            Conversation.data["cron_hidden"].astext.is_distinct_from("true"),
        )
    if folder_id is not None:
        statement = statement.where(Conversation.data["folder_id"].astext == folder_id)
    if project_id is not None:
        statement = statement.where(Conversation.data["project_id"].astext == project_id)
    query = q.strip()
    if query:
        matching_messages = select(Message.conversation_id).where(
            Message.owner_id == s.owner_id,
            Message.data["content"].astext.icontains(query, autoescape=True),
        )
        statement = statement.where(
            or_(
                Conversation.data["title"].astext.icontains(query, autoescape=True),
                Conversation.id.in_(matching_messages),
            )
        )
    last_activity = func.coalesce(
        Conversation.data["last_message_at"].astext,
        func.to_char(Conversation.created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US'),
    )
    pinned = case((Conversation.data["pinned"].astext == "true", 1), else_=0)
    statement = statement.order_by(pinned.desc(), last_activity.desc(), Conversation.id.desc())
    if offset:
        statement = statement.offset(offset)
    if limit is not None:
        statement = statement.limit(limit)
    return [public(row) for row in db.scalars(statement)]


@router.delete("/projects/{key}")
def delete_project(key: str, ctx=Depends(context)):
    db, s = ctx
    project = own(db, Project, key, s.owner_id)
    for conversation in db.scalars(
        select(Conversation).where(
            Conversation.owner_id == s.owner_id,
            Conversation.data["project_id"].astext == key,
        )
    ):
        conversation.data = {**conversation.data, "project_id": None}
    db.delete(project)
    audit(db, s.owner_id, "project.delete", key)
    db.commit()
    return {"ok": True}


@router.patch("/conversations/{key}/state")
def set_conversation_state(key: str, body: ConversationStateInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Conversation, key, s.owner_id)
    values = body.model_dump(exclude_unset=True)
    if values.get("folder_id"):
        own(db, ConversationFolder, values["folder_id"], s.owner_id)
    if values.get("project_id"):
        own(db, Project, values["project_id"], s.owner_id)
    data = dict(row.data)
    if "folder_id" in values:
        data["folder_id"] = values["folder_id"]
    if "project_id" in values:
        data["project_id"] = values["project_id"]
    if "pinned" in values:
        data["pinned"] = values["pinned"]
    if "archived" in values:
        data["archived_at"] = now().isoformat() if values["archived"] else None
    row.data = data
    audit(db, s.owner_id, "conversation.state", key)
    db.commit()
    return public(row)


@router.post("/conversations/{key}/restore")
def restore_conversation(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Conversation, key, s.owner_id)
    row.data = {**row.data, "deleted_at": None}
    audit(db, s.owner_id, "conversation.restore", key)
    db.commit()
    return public(row)


@router.delete("/conversations/{key}")
async def delete_conversation(key: str, permanent: bool = False, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Conversation, key, s.owner_id)
    stopped_runs = []
    if permanent:
        if not row.data.get("deleted_at"):
            raise HTTPException(409, "ごみ箱に移動してから完全削除してください")
        purge_conversation(db, row)
        audit(db, s.owner_id, "conversation.purge", key)
    else:
        for run in db.scalars(
            select(Run).where(
                Run.conversation_id == key,
                Run.status.in_(["queued", "running", "waiting_approval", "paused"]),
            )
        ):
            stopped_runs.append(run.id)
            if run.id in TASKS:
                TASKS[run.id].cancel()
            transition_run(db, run, "cancelled", reason="会話がごみ箱へ移動しました")
        row.data = {**row.data, "deleted_at": now().isoformat(), "pinned": False}
        audit(db, s.owner_id, "conversation.trash", key)
    db.commit()
    for stopped_id in stopped_runs:
        try:
            await runner_request("browser", "/browser-close", {"run_id": stopped_id}, timeout=5)
        except Exception:  # noqa: BLE001, S110 - the browser may already be closed
            pass
    return {"ok": True}


@router.get("/conversations/{key}/markdown")
def export_conversation_markdown(
    key: str, scope: Literal["answers", "conversation", "activity"] = "conversation", ctx=Depends(context)
):
    db, s = ctx
    row = own(db, Conversation, key, s.owner_id)
    lines = [
        f"# {row.data.get('title', '新しいチャット')}",
        "",
        f"作成: {row.created_at.isoformat()}",
        f"書き出し範囲: {scope}",
        "",
    ]
    for message in db.scalars(
        select(Message).where(Message.conversation_id == key).order_by(Message.created_at)
    ):
        if scope == "answers" and message.data.get("role") != "assistant":
            continue
        lines += [
            "## " + ("あなた" if message.data.get("role") == "user" else "MIX agent"),
            "",
            message.data.get("content", ""),
            "",
        ]
        if scope != "answers":
            for artifact in message.data.get("artifacts", []):
                lines.append(f"添付: {artifact.get('name', 'attachment')}")
    if scope == "activity":
        run_ids = list(db.scalars(select(Run.id).where(Run.conversation_id == key)))
        if run_ids:
            lines += ["## Tool の実行概要", "", "引数・結果本文・資格情報は含みません。", ""]
            for call in db.scalars(
                select(ToolCall).where(ToolCall.run_id.in_(run_ids)).order_by(ToolCall.created_at)
            ):
                label = (call.data.get("activity") or {}).get("label") or call.data.get("tool_id") or "Tool"
                lines.append(f"- {call.created_at.isoformat()} · {label} · {call.status}")
    safe_name = "conversation-" + key + ".md"
    return PlainTextResponse(
        "\n".join(lines), headers={"Content-Disposition": f'attachment; filename="{safe_name}"'}
    )


@router.get("/conversations/{key}/messages", response_model=ConversationMessagesView)
def messages(key: str, ctx=Depends(context)):
    db, s = ctx
    conversation = own(db, Conversation, key, s.owner_id)
    rows = [
        public(r)
        for r in db.scalars(
            select(Message).where(Message.conversation_id == key).order_by(Message.created_at)
        )
    ]
    message_ids = [row["id"] for row in rows]
    feedback = {
        item.message_id: item.data.get("value")
        for item in (
            db.scalars(
                select(Feedback).where(
                    Feedback.owner_id == s.owner_id,
                    Feedback.message_id.in_(message_ids),
                )
            )
            if message_ids
            else []
        )
    }
    for row in rows:
        if row["id"] in feedback:
            row["data"]["feedback"] = feedback[row["id"]]
    run_rows = list(db.scalars(select(Run).where(Run.conversation_id == key).order_by(Run.created_at)))
    user_turns = [
        {"id": row["id"], "created_at": datetime.fromisoformat(row["created_at"])}
        for row in rows
        if row["data"].get("role") == "user"
    ]

    def _legacy_message_id(r):
        stored = r.data.get("input_message_id")
        if stored:
            return stored
        # Legacy runs cannot be paired by list index: message lists can diverge
        # from run order after restore, deletions, or drafts. Pair with the
        # nearest user message created at-or-before the run instead. This is a
        # safety net; a data migration backfills the field for old rows.
        best = None
        for turn in user_turns:
            if turn["created_at"] <= r.created_at:
                best = turn["id"]
            else:
                break
        return best

    runs = [
        {
            "id": r.id,
            "status": r.status,
            "reason": r.data.get("reason"),
            "message_id": _legacy_message_id(r),
            "context_summary": r.data.get("summary") if (r.data.get("summary") or {}).get("text") else None,
            "answer_evaluation": r.data.get("answer_evaluation"),
        }
        for r in run_rows
    ]
    return {
        "messages": rows,
        "runs": runs,
        "selection": conversation.data.get("selection"),
        "project_id": conversation.data.get("project_id"),
        "todos": conversation_todos.sanitize(conversation.data.get("todos")),
    }


def _redact_tool_result(value):
    """Do not expose likely credentials in the user-facing inspection payload."""
    if isinstance(value, dict):
        return {
            key: "[redacted]"
            if any(
                part in key.lower() for part in ("secret", "token", "password", "api_key", "authorization")
            )
            else _redact_tool_result(item)
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [_redact_tool_result(item) for item in value]
    return value


@router.get("/conversations/{key}/tool-calls", response_model=list[ToolCallHistoryView])
def conversation_tool_calls(key: str, ctx=Depends(context)):
    db, s = ctx
    own(db, Conversation, key, s.owner_id)
    runs = list(db.scalars(select(Run).where(Run.conversation_id == key).order_by(Run.created_at)))
    run_map = {run.id: run for run in runs}
    approvals = (
        {
            approval.tool_call_id: approval
            for approval in db.scalars(select(Approval).where(Approval.run_id.in_(run_map)))
        }
        if run_map
        else {}
    )
    calls = (
        list(db.scalars(select(ToolCall).where(ToolCall.run_id.in_(run_map)).order_by(ToolCall.created_at)))
        if run_map
        else []
    )
    result = []
    for call in calls:
        run = run_map[call.run_id]
        data = call.data
        raw_result = data.get("result")
        failed = isinstance(raw_result, dict) and bool(raw_result.get("error"))
        unknown = run.status == "interrupted" and call.status == "executing"
        approval = approvals.get(call.id)
        state = (
            "unknown"
            if unknown
            else "waiting_approval"
            if call.status == "waiting_approval"
            else "running"
            if call.status in ("pending", "executing")
            else "failed"
            if failed
            else "completed"
        )
        retry = (
            {"available": True, "label": "外部状態を確認後、この実行を再開できます"}
            if unknown
            else {
                "available": False,
                "label": "個別再実行は未対応です。必要なら新しいメッセージで再依頼してください。",
            }
        )
        result.append(
            {
                "id": call.id,
                "run_id": call.run_id,
                "status": state,
                "tool_name": data.get("name", data.get("tool_id", "Tool")),
                "activity": data.get("activity"),
                "result_activity": data.get("result_activity"),
                "risk": data.get("risk"),
                "created_at": call.created_at.isoformat(),
                "result": _redact_tool_result(raw_result) if raw_result is not None else None,
                "failure": raw_result.get("error") if failed and isinstance(raw_result, dict) else None,
                "artifact": raw_result.get("artifact")
                if isinstance(raw_result, dict) and is_user_artifact(raw_result.get("artifact"))
                else None,
                "approval": (
                    {
                        "id": approval.id,
                        "status": approval.status,
                        "tool": approval.data.get("tool"),
                        "risk": approval.data.get("risk"),
                    }
                    if approval and approval.status == "pending"
                    else None
                ),
                "retry": retry,
            }
        )
    return result
