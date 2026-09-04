from fastapi.testclient import TestClient
from sqlalchemy import select

from mix_agent.auth.security import passwords
from mix_agent.db.models import Session, User
from mix_agent.db.session import SessionLocal
from mix_agent.main import app
from mix_agent.api.routes import record_login


def test_password_change_requires_current_password_and_rotates_csrf(signed):
    before = signed.headers["x-csrf-token"]
    bad = signed.post(
        "/api/v1/auth/password",
        json={"current_password": "wrong-password-123", "new_password": "new-password-12345"},
    )
    assert bad.status_code == 401
    changed = signed.post(
        "/api/v1/auth/password",
        json={"current_password": "test-password-12345", "new_password": "new-password-12345"},
    )
    assert changed.status_code == 200
    assert changed.json()["csrf"] != before
    signed.headers["x-csrf-token"] = changed.json()["csrf"]
    assert signed.get("/api/v1/auth/me").status_code == 200
    signed.post("/api/v1/auth/logout")
    assert signed.post("/api/v1/auth/login", json={"username": "tester", "password": "new-password-12345"}).status_code == 200


def test_password_change_can_revoke_every_session(signed):
    result = signed.post(
        "/api/v1/auth/password",
        json={"current_password": "test-password-12345", "new_password": "new-password-12345", "revoke_all_sessions": True},
    )
    assert result.status_code == 200
    assert result.json()["relogin_required"] is True
    assert signed.get("/api/v1/auth/me").status_code == 401
    with SessionLocal() as db:
        assert not db.scalars(select(Session)).all()


def test_username_change_rejects_duplicates_and_updates_me(signed):
    with SessionLocal() as db:
        db.add(User(username="taken", password_hash=passwords.hash("other-password-12345"), singleton=2))
        db.commit()
    duplicate = signed.post(
        "/api/v1/auth/username",
        json={"current_password": "test-password-12345", "username": "taken"},
    )
    assert duplicate.status_code == 409
    changed = signed.post(
        "/api/v1/auth/username",
        json={"current_password": "test-password-12345", "username": "renamed"},
    )
    assert changed.status_code == 200
    assert signed.get("/api/v1/auth/me").json()["username"] == "renamed"


def test_session_revocation_and_login_history_keep_only_safe_recent_events(signed):
    other = TestClient(app)
    login = other.post("/api/v1/auth/login", json={"username": "tester", "password": "test-password-12345"})
    assert login.status_code == 200
    assert signed.post("/api/v1/auth/sessions/revoke", json={"scope": "others"}).status_code == 200
    assert signed.get("/api/v1/auth/me").status_code == 200
    assert other.get("/api/v1/auth/me").status_code == 401
    with SessionLocal() as db:
        user = db.scalar(select(User).where(User.username == "tester"))
        for _ in range(101):
            record_login(db, user.id, False)
        db.commit()
    history = signed.get("/api/v1/auth/login-history")
    assert history.status_code == 200
    assert len(history.json()) == 100
    assert all(set(event) == {"successful", "created_at"} for event in history.json())
    assert any(not event["successful"] for event in history.json())


def test_account_endpoints_require_authentication_and_csrf(signed):
    anonymous = TestClient(app)
    assert anonymous.post("/api/v1/auth/sessions/revoke", json={"scope": "all"}).status_code == 401
    result = signed.post("/api/v1/auth/sessions/revoke", json={"scope": "others"}, headers={"x-csrf-token": "wrong"})
    assert result.status_code == 403
