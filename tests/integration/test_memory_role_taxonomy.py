"""ChatGPT-style runtime tests: remember / recall / forget (flat)."""

from __future__ import annotations

from sqlalchemy import func, select

from mix_agent.db.models import Memory, User
from mix_agent.db.session import SessionLocal
from mix_agent.memory import runtime as mem_runtime
from mix_agent.memory import tools as mem_tools


def owner_id(db):
    return db.scalar(select(User.id))


def test_remember_dedups_instead_of_growing(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        first = mem_tools.remember(db, owner, content="回答は日本語にする", explicit_user=True)
        db.commit()
        second = mem_tools.remember(db, owner, content="回答は日本語にする", explicit_user=True)
        db.commit()
        assert first["applied"]
        assert second["decision"] == "duplicate"
        total = db.scalar(select(func.count()).select_from(Memory).where(Memory.owner_id == owner))
        assert total == 1


def test_recall_finds_saved_fact(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        mem_tools.remember(db, owner, content="MIX-agent は PostgreSQL を使う", explicit_user=True)
        mem_tools.remember(db, owner, content="回答は日本語にする", explicit_user=True)
        db.commit()
        result = mem_runtime.recall(db, owner, {"text": "どのDBを使う？ PostgreSQL", "kind": "test"}, limit=8)
        assert result
        assert any("PostgreSQL" in row["content"] for row in result)


def test_forget_deletes_row(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        created = mem_tools.remember(db, owner, content="一時的なメモ", explicit_user=True)
        db.commit()
        mid = next(item["id"] for item in created["applied"] if item.get("action") == "create")
        out = mem_tools.forget(db, owner, memory_id=mid)
        db.commit()
        assert out["forgotten"] == [mid]
        assert db.get(Memory, mid) is None


def test_recall_debug_shape(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        mem_tools.remember(
            db, owner, content="Postgres のマイグレーションはDocker経由で実施する", explicit_user=True
        )
        db.commit()
        debug = mem_runtime.recall(
            db, owner, {"text": "マイグレーションを実行したい", "kind": "test"}, debug=True
        )
        assert "memories" in debug and debug["memories"]
        assert debug["debug"]["weights"]["lexical"] >= 0.0


def test_group_for_context_flat(signed):
    grouped = mem_runtime.group_for_context([{"id": "a", "content": "x"}])
    assert [row["id"] for row in grouped["all"]] == ["a"]
