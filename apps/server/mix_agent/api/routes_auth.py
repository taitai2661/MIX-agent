"""Setup and account/session endpoints.

Extracted from ``api.routes`` so the authentication surface has its own module
and tests can target it without importing the whole API.  Route paths and
response shapes are unchanged.
"""

from fastapi import APIRouter, Depends, Request, Response
from sqlalchemy import select
from sqlalchemy.exc import IntegrityError

from mix_agent.api.common import audit, context, record_login, revoke_sessions
from mix_agent.api.error_codes import APIError
from mix_agent.api.schemas import (
    Credentials,
    PasswordChangeInput,
    SessionRevocationInput,
    UsernameChangeInput,
)
from mix_agent.auth.security import (
    ACCOUNT_CHANGE_LIMIT,
    ACCOUNT_CHANGE_WINDOW_SECONDS,
    LOGIN_ACCOUNT_LIMIT,
    LOGIN_IP_LIMIT,
    LOGIN_WINDOW_SECONDS,
    clear_attempts,
    dummy_password_hash,
    new_session,
    new_session_csrf,
    passwords,
    record_failed_attempt,
    throttle_attempt,
)
from mix_agent.client_ip import request_client_ip
from mix_agent.db.models import LoginEvent, Settings, User, Workspace
from mix_agent.db.session import get_db

router = APIRouter(prefix="/api/v1")


@router.get("/setup")
def setup(db=Depends(get_db)):
    return {"needs_admin": db.scalar(select(User.id)) is None}


@router.post("/setup/admin")
def setup_admin(body: Credentials, response: Response, db=Depends(get_db)):
    if db.scalar(select(User.id)):
        raise APIError(409, "setup.already_complete")
    user = User(username=body.username, password_hash=passwords.hash(body.password))
    db.add(user)
    try:
        db.flush()
        session = new_session(db, user.id, response)
        db.add(
            Settings(
                id="settings",
                owner_id=user.id,
                data={
                    "setup_complete": False,
                    "allowed_domains": [],
                    "auto_retry_count": 3,
                },
            )
        )
        db.add(Workspace(owner_id=user.id, data={"name": "Workspace", "path": "/workspace"}))
        db.commit()
    except IntegrityError:
        db.rollback()
        raise APIError(409, "setup.admin_exists")
    return {"username": user.username, "csrf": session.csrf}


@router.post("/auth/login")
def login(body: Credentials, request: Request, response: Response, db=Depends(get_db)):
    ip = request_client_ip(request)
    user = db.scalar(select(User).where(User.username == body.username))
    ip_key = "login-ip:" + ip
    account_key = f"login:{user.id}" if user else None
    throttle_attempt(
        ip_key,
        LOGIN_IP_LIMIT,
        LOGIN_WINDOW_SECONDS,
    )
    if account_key:
        throttle_attempt(
            account_key,
            LOGIN_ACCOUNT_LIMIT,
            LOGIN_WINDOW_SECONDS,
        )
    try:
        if user:
            valid = bool(passwords.verify(user.password_hash, body.password))
        else:
            # Unknown username: still run a real argon2 verification so the
            # response time does not reveal whether the account exists.
            passwords.verify(dummy_password_hash(), body.password)
            valid = False
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        valid = False
    if not valid:
        # Failures only consume the throttle budget; a successful login must
        # never exhaust the bucket with its own legitimate traffic.
        record_failed_attempt(ip_key, LOGIN_WINDOW_SECONDS)
        if account_key:
            record_failed_attempt(account_key, LOGIN_WINDOW_SECONDS)
            record_login(db, user.id, False, ip)
            db.commit()
        raise APIError(401, "auth.invalid_credentials")
    # A valid password clears this account's failure budget.  The IP bucket is
    # intentionally left alone: resetting it on any successful login would let
    # an attacker with one valid account clear the address-wide brute-force
    # protection for every other account on the same address.
    clear_attempts(account_key)
    session = new_session(db, user.id, response)
    record_login(db, user.id, True, ip)
    db.commit()
    return {"username": user.username, "csrf": session.csrf}


@router.get("/auth/me")
def me(ctx=Depends(context)):
    db, session = ctx
    return {"username": db.get(User, session.owner_id).username, "csrf": session.csrf}


@router.post("/auth/logout")
def logout(response: Response, ctx=Depends(context)):
    db, session = ctx
    db.delete(session)
    db.commit()
    response.delete_cookie("mix_session")
    return {"ok": True}


@router.post("/auth/password")
def change_password(body: PasswordChangeInput, response: Response, ctx=Depends(context)):
    db, session = ctx
    change_key = "password-change:" + session.owner_id
    throttle_attempt(change_key, ACCOUNT_CHANGE_LIMIT, ACCOUNT_CHANGE_WINDOW_SECONDS)
    user = db.get(User, session.owner_id)
    try:
        valid = passwords.verify(user.password_hash, body.current_password)
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        valid = False
    if not valid:
        record_failed_attempt(change_key, ACCOUNT_CHANGE_WINDOW_SECONDS)
        raise APIError(401, "auth.current_password_invalid")
    clear_attempts(change_key)
    user.password_hash = passwords.hash(body.new_password)
    audit(db, user.id, "password_changed", "account")
    if body.revoke_all_sessions:
        revoke_sessions(db, user.id)
        db.commit()
        response.delete_cookie("mix_session")
        return {"ok": True, "relogin_required": True}
    session.csrf = new_session_csrf()
    db.commit()
    return {"ok": True, "csrf": session.csrf, "relogin_required": False}


@router.post("/auth/username")
def change_username(body: UsernameChangeInput, ctx=Depends(context)):
    db, session = ctx
    change_key = "username-change:" + session.owner_id
    throttle_attempt(change_key, ACCOUNT_CHANGE_LIMIT, ACCOUNT_CHANGE_WINDOW_SECONDS)
    user = db.get(User, session.owner_id)
    try:
        valid = passwords.verify(user.password_hash, body.current_password)
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        valid = False
    if not valid:
        record_failed_attempt(change_key, ACCOUNT_CHANGE_WINDOW_SECONDS)
        raise APIError(401, "auth.current_password_invalid")
    clear_attempts(change_key)
    existing = db.scalar(select(User).where(User.username == body.username))
    if existing and existing.id != user.id:
        raise APIError(409, "auth.username_taken")
    user.username = body.username
    audit(db, user.id, "username_changed", "account")
    db.commit()
    return {"username": user.username, "csrf": session.csrf}


@router.post("/auth/sessions/revoke")
def revoke_account_sessions(body: SessionRevocationInput, response: Response, ctx=Depends(context)):
    db, session = ctx
    revoke_sessions(db, session.owner_id, None if body.scope == "all" else session.id)
    audit(db, session.owner_id, "sessions_revoked", body.scope)
    db.commit()
    if body.scope == "all":
        response.delete_cookie("mix_session")
    return {"ok": True, "relogin_required": body.scope == "all"}


@router.get("/auth/login-history")
def login_history(ctx=Depends(context)):
    db, session = ctx
    rows = db.scalars(
        select(LoginEvent)
        .where(LoginEvent.owner_id == session.owner_id)
        .order_by(LoginEvent.created_at.desc(), LoginEvent.id.desc())
        .limit(100)
    )
    return [{"successful": row.successful, "created_at": row.created_at.isoformat()} for row in rows]
