"""ChatGPT-style flat memory tests: CRUD + dedup + hard delete."""

from mix_agent.db.models import Memory, User
from mix_agent.db.session import SessionLocal
from mix_agent.memory import service
from sqlalchemy import func, select


def owner_id(db):
    return db.scalar(select(User.id))


def test_create_and_dedup(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        first = service.change(db, owner, "回答は日本語にする", source_run="explicit-user-request")
        assert first["content"] == "回答は日本語にする"
        second = service.change(db, owner, "回答は日本語にする", source_run="explicit-user-request")
        db.commit()
        assert second["deduplicated"] is True
        assert db.scalar(select(func.count()).select_from(Memory).where(Memory.owner_id == owner)) == 1


def test_search_finds_by_text_and_ignores_unrelated(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        service.change(db, owner, "回答は日本語にする", source_run="explicit-user-request")
        db.commit()
        assert service.search(db, owner, "日本語") != []
        assert service.search(db, owner, "南極のペンギンの個体数") == []


def test_update_and_hard_delete(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        created = service.change(db, owner, "OSSを優先する", source_run="explicit-user-request")
        updated = service.change(db, owner, "OSSを最優先する", memory_id=created["id"])
        assert updated["content"] == "OSSを最優先する"
        service.change(db, owner, memory_id=created["id"], delete=True)
        db.commit()
        assert db.get(Memory, created["id"]) is None


def test_secret_rejected_and_forget_parser():
    assert "private-value" not in service.redact_sensitive("api_key=private-value")
    assert service.explicit_forget_candidate("Chromeをメインに使うを忘れて") == "Chromeをメインに使う"
    try:
        import sqlalchemy as _sa  # noqa: F401
    except Exception:
        pass
