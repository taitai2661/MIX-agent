"""Idempotency-key lookup shared by the message send/edit endpoints."""

from fastapi import HTTPException
from sqlalchemy import select

from mix_agent.db.models import Run


def find_existing_run(db, request_key, owner_id, conversation_id, request_hash):
    """Return the Run for an Idempotency-Key, validating who/what it belongs to.

    Returns ``None`` when the key has not been seen. A caller that expected to
    insert must distinguish that from a concurrent winner; without these checks
    a duplicate key could hand one conversation (or owner) another Run and then
    launch it.
    """
    existing = db.scalar(select(Run).where(Run.request_key == request_key))
    if existing is None:
        return None
    if existing.owner_id != owner_id or existing.conversation_id != conversation_id:
        raise HTTPException(409)
    if existing.data.get("request_hash") != request_hash:
        raise HTTPException(409, "同じIdempotency-Keyで異なる入力は送信できません")
    return existing
