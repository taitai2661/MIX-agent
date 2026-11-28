"""Bounded reads for the conversation history endpoints.

The transcript and the tool-call history of a long conversation used to be
returned in full: every message, every run and every tool call of it, with
each tool call carrying its (redacted) result. A conversation that has been
open for months therefore produced a multi-megabyte response and a large
allocation on every poll.

Both endpoints now read the **newest** page and return it oldest-first, which
is the order the UI renders. The cap is the same for both editions, so the
Docker and Desktop histories stop at the same place.

The helpers live here rather than in the route modules so the route files
stay within their line budgets: the budget test treats a growing router as a
signal to extract the next cluster, not to raise the number.
"""

from __future__ import annotations

from sqlalchemy import select

from mix_agent.db.models import Approval, Memory, Run, ToolCall

# How many messages of one conversation the transcript endpoint returns at
# most.
MESSAGE_PAGE_LIMIT = 500

# How many tool calls of one conversation the history endpoint returns at
# most. The Desktop edition caps its history at the same number.
TOOL_CALL_PAGE_LIMIT = 2000

# How many revisions of one memory the history endpoint returns. The log is
# append-only, so an unbounded read grew with every edit of a long-lived
# memory.
MEMORY_REVISION_LIMIT = 200


def page_cap(limit: int | None, default: int) -> int:
    """Clamp a caller-supplied page size into ``1..default``."""
    try:
        requested = int(limit or 0)
    except (TypeError, ValueError):
        requested = 0
    return max(1, min(requested or default, default))


def conversation_messages(db, conversation_id: str, cap: int):
    """The newest ``cap`` messages of a conversation, oldest first."""
    from mix_agent.db.models import Message

    rows = list(
        db.scalars(
            select(Message)
            .where(Message.conversation_id == conversation_id)
            .order_by(Message.created_at.desc(), Message.id.desc())
            .limit(cap)
        )
    )
    rows.reverse()
    return rows


def conversation_tool_calls(db, conversation_id: str, cap: int):
    """The newest ``cap`` tool calls of a conversation, oldest first.

    The cap is applied to the *calls* rather than to the runs: capping the
    runs would silently drop every call of the older runs, which is a
    different list than "the newest N calls".
    """
    run_ids = select(Run.id).where(Run.conversation_id == conversation_id)
    calls = list(
        db.scalars(
            select(ToolCall)
            .where(ToolCall.run_id.in_(run_ids))
            .order_by(ToolCall.created_at.desc(), ToolCall.id.desc())
            .limit(cap)
        )
    )
    calls.reverse()
    return calls


def runs_for_tool_calls(db, run_ids: set[str]):
    """The runs the given tool calls belong to."""
    if not run_ids:
        return {}
    return {run.id: run for run in db.scalars(select(Run).where(Run.id.in_(run_ids)))}


def approvals_for_tool_calls(db, run_ids: set[str]):
    """The approvals of the given runs, keyed by the tool call they answer."""
    if not run_ids:
        return {}
    return {
        approval.tool_call_id: approval
        for approval in db.scalars(select(Approval).where(Approval.run_id.in_(run_ids)))
    }


def memory_created_at(db, owner_id: str, ids: list[str]) -> dict:
    """The creation time of the given memories, keyed by id.

    Only the ids on one page are read, and only their timestamp: the listing
    used to load *every* Memory row of the owner into a dict, which grew
    without bound on a long-lived account.
    """
    if not ids:
        return {}
    return {
        row.id: row.created_at
        for row in db.execute(
            select(Memory.id, Memory.created_at).where(
                Memory.owner_id == owner_id,
                Memory.id.in_(ids),
            )
        ).all()
    }


def memory_revisions(db, owner_id: str, memory_id: str, cap: int):
    """The newest revisions of one memory, newest first.

    The revision log is append-only, so an unbounded read grew with every
    edit of a long-lived memory.
    """
    from mix_agent.db.models import MemoryRevision

    return list(
        db.scalars(
            select(MemoryRevision)
            .where(
                MemoryRevision.owner_id == owner_id,
                MemoryRevision.data["memory_id"].as_string() == memory_id,
            )
            .order_by(MemoryRevision.created_at.desc())
            .limit(cap)
        )
    )
