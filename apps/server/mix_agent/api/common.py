"""Shared request helpers used by more than one API router.

Keeping these here (rather than in ``routes.py``) lets an extracted router such
as ``routes_auth`` depend on them without importing the 4,000-line module and
recreating a cycle.
"""

import re
from typing import TypeVar

from fastapi import Depends, HTTPException, Request
from sqlalchemy import delete, select

from mix_agent.auth.security import authenticate
from mix_agent.db.models import Audit, LoginEvent, Session
from mix_agent.db.session import get_db

T = TypeVar("T")


def context(request: Request, db=Depends(get_db)):
    return db, authenticate(request, db)


def own(db, cls: type[T], key, owner) -> T:
    """Fetch one owned row or raise 404.

    Typed as ``T`` so callers can rely on the row after the ownership check;
    the untyped version made every attribute access a pyright warning.
    """
    row = db.get(cls, key)
    if not row or row.owner_id != owner:
        raise HTTPException(404, "見つかりません")
    return row


def public(row):
    data = dict(row.data)
    for key in ("secret_id", "brave_secret_id", "tavily_secret_id", "exa_secret_id", "serper_secret_id"):
        if key in data:
            data["has_" + key] = bool(data.pop(key))
    return {"id": row.id, "data": data, "created_at": row.created_at.isoformat()}


def audit(db, owner, action, target):
    db.add(Audit(owner_id=owner, data={"action": action, "target": target}))


def skill_filename(data):
    base = re.sub(r"[^A-Za-z0-9_-]+", "-", str(data.get("source_slug") or data.get("name") or "skill")).strip(
        "-"
    )
    return base or "skill"


def conversation_title(content):
    text = " ".join(content.strip().split())
    return (text[:77] + "…") if len(text) > 78 else (text or "新しいチャット")


def set_title_placeholder(value, explicit: bool):
    """Record whether the title is the generated placeholder.

    Comparing the title to a Japanese default string treated a conversation the
    user deliberately renamed to that text as untouched. New rows carry the
    decision explicitly; ``title_is_placeholder`` reads the legacy shape only
    when the flag is absent.
    """
    value["title_is_placeholder"] = not explicit


def title_is_placeholder(data) -> bool:
    return bool(data.get("title_is_placeholder", data.get("title") == "新しいチャット"))


def with_generated_title(data, content):
    """Return ``data`` with the first user message used as its title.

    Leaves an explicitly named conversation untouched, including one the user
    renamed to the placeholder text.
    """
    if not title_is_placeholder(data):
        return data
    return {
        **data,
        # An attachment-only turn can have whitespace where text would be
        # (`MessageInput` allows it when artifacts are present). It still needs
        # a real generated title, not the placeholder marked as final.
        "title": conversation_title((content or "").strip() or "添付について"),
        "title_is_placeholder": False,
    }


def record_login(db, owner_id, successful, ip="unknown"):
    """Keep only the latest 100 outcomes for this account, with connection metadata."""
    db.add(LoginEvent(owner_id=owner_id, successful=successful, ip=ip))
    db.flush()
    stale_ids = (
        select(LoginEvent.id)
        .where(LoginEvent.owner_id == owner_id)
        .order_by(LoginEvent.created_at.desc(), LoginEvent.id.desc())
        .offset(100)
    )
    db.execute(delete(LoginEvent).where(LoginEvent.id.in_(stale_ids)))


def revoke_sessions(db, owner_id, current_session_id=None):
    rows = db.scalars(select(Session).where(Session.owner_id == owner_id))
    for row in rows:
        if current_session_id is None or row.id != current_session_id:
            db.delete(row)
