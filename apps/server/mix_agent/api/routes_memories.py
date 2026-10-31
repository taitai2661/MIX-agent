"""Memory endpoints split out of ``routes.py``.

Kept in their own module so the main router file stays reviewable. The routes
are mounted on the same application prefix; no URL changed.
"""

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select

from mix_agent.api.common import context, own, public
from mix_agent.api.schemas import (
    MemoryInput,
    MemoryResolveConflictInput,
    MemoryResolveConflictView,
)
from mix_agent.auth.security import read_secret
from mix_agent.db.models import Memory, MemoryActionEvent, MemoryRevision
from mix_agent.memory import embeddings as mem_embeddings
from mix_agent.memory import runtime as memory_runtime
from mix_agent.memory import service as memory

router = APIRouter(prefix="/api/v1")


@router.get("/memories")
def memories(q: str = "", state: str | None = None, ctx=Depends(context)):
    """Flat listing. ``state`` narrows it to one §2 ``lifecycle_state``.

    A value outside §2 is a **422**, not an empty list: answering "nothing
    matched" for a typo is the same failure mode as ignoring ``state``
    outright, which is what this parameter used to do.
    """
    db, s = ctx
    try:
        rows = memory.list_traces(db, s.owner_id, q, state)
    except ValueError as exc:
        raise HTTPException(422, str(exc))
    records = {row.id: row for row in db.scalars(select(Memory).where(Memory.owner_id == s.owner_id))}
    # Spec §4: how many edges touch each row, counted once for the whole page.
    # Same convention `views.render` uses — the key appears only when the count
    # is non-zero, so a store without links answers exactly what it did before
    # §4 existed (additive, never a new required key).
    counts = memory.link_counts(db, s.owner_id, [item["id"] for item in rows])
    return [
        {
            "id": item["id"],
            "data": {
                **{k: v for k, v in item.items() if k != "id"},
                **({"link_count": counts[item["id"]]} if counts.get(item["id"]) else {}),
            },
            "created_at": records[item["id"]].created_at.isoformat() if item["id"] in records else "",
        }
        for item in rows
        if item["id"] in records
    ]


@router.post("/memories/{key}/resolve", response_model=MemoryResolveConflictView)
def resolve_memory_conflict(key: str, body: MemoryResolveConflictInput, ctx=Depends(context)):
    """Spec §3 ``resolve_conflict`` over HTTP: settle a dispute in place.

    The UI reaches this from a row that §1 already reports as ``disputed``.
    Everything the settlement does stays §3's: the adopted row goes back to
    ``active``, the rejected one becomes ``superseded`` (``expired`` for
    ``dismiss``) and points at the winner, both rows keep a revision first, and
    §4's mirrors follow in the same transaction.

    ``prefer`` is passed through untouched and **left alone when the client
    does not send it**: §3's default — the memory ``disputed_by_id`` points at
    is the one already stored, and it wins — is decided here, not in the form.
    """
    db, s = ctx
    own(db, Memory, key, s.owner_id)
    try:
        result = memory_runtime.resolve_conflict(
            db, s.owner_id, conflict_id=key, resolution=body.resolution, prefer=body.prefer
        )
    except ValueError as exc:
        raise HTTPException(422, str(exc))
    db.commit()
    return {
        "decision": "resolved",
        "id": result["winner"],
        "reason": result["resolution"],
        "loser_id": result["loser"],
        "loser_state": result["loser_state"],
    }


@router.get("/memories/{key}/associations")
def memory_associations(key: str, ctx=Depends(context)):
    """Spec §4: the `memory_links` edges touching one memory (§9 #3).

    Both directions, owner-scoped through the same `own()` guard every other
    `/memories/{key}` route uses, so an edge can never be enumerated by
    guessing another owner's ids. Returns the flat edge dicts (source,
    target, kind, weight, coactivation_count) — §4 is the contract for the
    shape, not the trace view.
    """
    db, s = ctx
    own(db, Memory, key, s.owner_id)
    return memory.associations_for(db, s.owner_id, key)


@router.get("/memories-debug/search")
async def debug_memory_search(q: str, ctx=Depends(context)):
    db, s = ctx
    provider = mem_embeddings.default_provider(db, s.owner_id)
    key = read_secret(db, provider.data.get("secret_id")) if provider else None
    vector = (
        await mem_embeddings.query_vector({**provider.data, "id": provider.id}, key, q) if provider else None
    )
    return memory.search(db, s.owner_id, q, debug=True, query_vector=vector)


@router.post("/memories-debug/reembed")
async def debug_reembed(limit: int = 50, ctx=Depends(context)):
    """Backfill embeddings for rows written before this feature existed.

    Kept behind the debug namespace: it spends provider quota, so it is an
    operator action rather than something the UI should trigger implicitly.
    """
    db, s = ctx
    return {"embedded": await mem_embeddings.embed_pending(db, s.owner_id, limit=max(1, min(200, limit)))}


@router.get("/memories-debug/actions")
def debug_memory_actions(run_id: str | None = None, ctx=Depends(context)):
    db, s = ctx
    statement = select(MemoryActionEvent).where(MemoryActionEvent.owner_id == s.owner_id)
    if run_id:
        statement = statement.where(MemoryActionEvent.run_id == run_id)
    return [
        public(row) for row in db.scalars(statement.order_by(MemoryActionEvent.created_at.desc()).limit(200))
    ]


@router.post("/memories")
async def add_memory(body: MemoryInput, ctx=Depends(context)):
    db, s = ctx
    try:
        result = memory.change(db, s.owner_id, source_run="explicit-user-request", **body.model_dump())
    except ValueError as exc:
        raise HTTPException(422, str(exc))
    db.commit()
    # Embedding is best-effort: a provider without an embeddings endpoint leaves
    # the row un-vectored and recall simply ranks it lexically.
    if await mem_embeddings.embed_memory(db, result["id"]):
        db.commit()
    return result


@router.patch("/memories/{key}")
async def edit_memory(key: str, body: MemoryInput, ctx=Depends(context)):
    db, s = ctx
    own(db, Memory, key, s.owner_id)
    try:
        result = memory.change(db, s.owner_id, memory_id=key, **body.model_dump())
    except ValueError as exc:
        raise HTTPException(422, str(exc))
    db.commit()
    # `change` drops the vector when the content changed; re-embed the new text.
    if await mem_embeddings.embed_memory(db, key):
        db.commit()
    return result


@router.get("/memories/{key}/revisions")
def memory_history(key: str, ctx=Depends(context)):
    db, s = ctx
    own(db, Memory, key, s.owner_id)
    rows = db.scalars(
        select(MemoryRevision)
        .where(MemoryRevision.owner_id == s.owner_id, MemoryRevision.data["memory_id"].as_string() == key)
        .order_by(MemoryRevision.created_at.desc())
    )
    return [public(r) for r in rows]


@router.post("/memories/{key}/restore/{revision}")
def restore_memory(key: str, revision: str, ctx=Depends(context)):
    db, s = ctx
    rev = own(db, MemoryRevision, revision, s.owner_id)
    revision_data = rev.data if isinstance(rev.data, dict) else {}
    if revision_data.get("memory_id") != key:
        raise HTTPException(404)
    previous = revision_data.get("previous")
    content = previous.get("content") or previous.get("gist") if isinstance(previous, dict) else None
    if not isinstance(content, str) or not content.strip():
        raise HTTPException(404, "復元できるMemory revisionがありません")
    row = db.get(Memory, key)
    if row is not None and row.owner_id != s.owner_id:
        raise HTTPException(404, "見つかりません")
    if row is None:
        try:
            row, restored = memory.restore_deleted(db, s.owner_id, key, previous)
        except (TypeError, ValueError) as exc:
            raise HTTPException(404, "復元できるMemory revisionがありません") from exc
    else:
        restored = memory.restore(db, s.owner_id, row, previous)
    db.commit()
    return {
        "id": row.id,
        "data": {k: v for k, v in restored.items() if k != "id"},
        "created_at": row.created_at.isoformat(),
    }
