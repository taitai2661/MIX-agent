"""Notifications, Web Push subscriptions and the read-only audit trail.

Extracted from ``api.routes`` so this small, mostly-crud cluster stops growing
the conversation/run module.  Route paths and response shapes are unchanged.
"""

from fastapi import APIRouter, Depends, HTTPException, Query
from sqlalchemy import delete, select

from mix_agent.api.common import context, own, public
from mix_agent.api.schemas import NotificationReadInput, RecordView
from mix_agent.db.models import Audit, Notification, PushSubscription, now

router = APIRouter(prefix="/api/v1")


@router.get("/notifications")
def list_notifications(unread: bool = False, ctx=Depends(context)):
    db, s = ctx
    query = select(Notification).where(Notification.owner_id == s.owner_id)
    if unread:
        query = query.where(Notification.read_at.is_(None))
    return [public(row) for row in db.scalars(query.order_by(Notification.created_at.desc()).limit(100))]


@router.patch("/notifications/{key}")
def read_notification(key: str, body: NotificationReadInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Notification, key, s.owner_id)
    row.read_at = now() if body.read else None
    db.commit()
    return public(row)


@router.get("/push/key")
def push_key(ctx=Depends(context)):
    from mix_agent.push import public_key

    return {"public_key": public_key()}


@router.get("/push/subscriptions")
def push_subscriptions(ctx=Depends(context)):
    db, s = ctx
    return {
        "endpoints": [
            row.endpoint
            for row in db.scalars(select(PushSubscription).where(PushSubscription.owner_id == s.owner_id))
        ]
    }


@router.post("/push/subscriptions")
def save_push_subscription(body: dict, ctx=Depends(context)):
    from mix_agent.push import unb64, valid_endpoint

    db, s = ctx
    endpoint = body.get("endpoint")
    keys = body.get("keys")
    if not isinstance(endpoint, str) or not valid_endpoint(endpoint) or not isinstance(keys, dict):
        raise HTTPException(422, "Push購読が不正です")
    try:
        if len(unb64(keys["p256dh"])) != 65 or len(unb64(keys["auth"])) != 16:
            raise ValueError()
    except (KeyError, TypeError, ValueError):
        raise HTTPException(422, "Push購読キーが不正です") from None
    row = db.scalar(select(PushSubscription).where(PushSubscription.endpoint == endpoint))
    if row and row.owner_id != s.owner_id:
        raise HTTPException(409, "別ユーザーの購読です")
    if not row:
        row = PushSubscription(owner_id=s.owner_id, endpoint=endpoint)
        db.add(row)
    row.data = {"p256dh": keys["p256dh"], "auth": keys["auth"]}
    db.commit()
    return {"enabled": True}


@router.delete("/push/subscriptions")
def delete_push_subscription(body: dict, ctx=Depends(context)):
    db, s = ctx
    endpoint = body.get("endpoint")
    if not isinstance(endpoint, str):
        raise HTTPException(422, "Push購読が不正です")
    db.execute(
        delete(PushSubscription).where(
            PushSubscription.owner_id == s.owner_id, PushSubscription.endpoint == endpoint
        )
    )
    db.commit()
    return {"enabled": False}


@router.get("/audit-events", response_model=list[RecordView])
def audit_events(
    action: str = "",
    limit: int = Query(default=100, ge=1, le=500),
    ctx=Depends(context),
):
    """Read-only audit trail.  Writes already existed; this exposes them."""
    db, s = ctx
    statement = select(Audit).where(Audit.owner_id == s.owner_id)
    if action.strip():
        statement = statement.where(Audit.data["action"].astext == action.strip())
    statement = statement.order_by(Audit.created_at.desc()).limit(limit)
    return [public(row) for row in db.scalars(statement)]
