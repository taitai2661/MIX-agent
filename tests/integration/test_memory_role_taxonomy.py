"""Tests for the role-based Memory Runtime.

The runtime is the orchestrator that powers the Agent Loop:
- recall pipeline (role-aware ranking + failure/decision boosts)
- evaluator (create / reinforce / supersede / dispute)
- lifecycle helpers (attach_evidence, supersede, dispute, resolve_conflict)
- context grouping (role buckets for Context Engine)

These tests use PostgreSQL via the shared ``signed`` fixture and exercise the
runtime against a real DB so the SQL joins / index paths are covered.
"""

from __future__ import annotations

from sqlalchemy import func, select

from mix_agent.db.models import Memory, MemoryConflict, MemoryEvidence, User
from mix_agent.db.session import SessionLocal
from mix_agent.memory import runtime as mem_runtime
from mix_agent.memory import tools as mem_tools
from mix_agent.memory import types as mem_types


def owner_id(db):
    return db.scalar(select(User.id))


def test_remember_reinforces_duplicate_instead_of_growing(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        first = mem_tools.remember(
            db, owner,
            content="回答は日本語にする",
            role=mem_types.ROLE_PREFERENCE,
            scope=mem_types.SCOPE_USER,
            explicit_user=True,
        )
        db.commit()
        second = mem_tools.remember(
            db, owner,
            content="回答は日本語にする",
            role=mem_types.ROLE_PREFERENCE,
            scope=mem_types.SCOPE_USER,
            explicit_user=True,
        )
        db.commit()
        assert first["applied"]
        assert second["decision"] == "duplicate"
        total = db.scalar(select(func.count()).select_from(Memory).where(Memory.owner_id == owner))
        assert total == 1


def test_recall_ranks_failure_above_unrelated_facts(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        mem_tools.remember(db, owner, content="MIX-agent は PostgreSQL を使う", role=mem_types.ROLE_FACT, scope=mem_types.SCOPE_USER, explicit_user=True)
        mem_tools.remember(
            db, owner,
            content="run_terminal で Docker migration を直接実行すると失敗する",
            role=mem_types.ROLE_FAILURE,
            scope=mem_types.SCOPE_USER,
            role_metadata={
                "attempt": "docker compose exec api alembic upgrade",
                "outcome": "failed",
                "reason": "依存パッケージが足りない",
                "lesson": "MIX 環境では migration は別ツール経由で行う",
                "related_tool": "run_terminal",
                "retry_suggested": False,
            },
            explicit_user=True,
        )
        mem_tools.remember(db, owner, content="MIX-agent は日本語UIが基本", role=mem_types.ROLE_PREFERENCE, scope=mem_types.SCOPE_USER, explicit_user=True)
        db.commit()
        result = mem_runtime.recall(
            db, owner,
            {"text": "Docker migration を再実行したい", "kind": "test", "tools_used": ["run_terminal"]},
            limit=8,
        )
        assert result
        ids = [row["id"] for row in result]
        failure_id = db.scalar(select(Memory.id).where(Memory.role == mem_types.ROLE_FAILURE, Memory.owner_id == owner))
        assert failure_id in ids
        # The failure memory should appear above a random unrelated fact.
        failure_index = ids.index(failure_id)
        fact_index = ids.index(next(i for i in ids if i != failure_id))
        assert failure_index < fact_index


def test_evaluator_opens_dispute_when_same_subject_disagrees(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        mem_tools.remember(
            db, owner,
            content="MIX-agent の主DBは PostgreSQL である",
            role=mem_types.ROLE_FACT,
            scope=mem_types.SCOPE_WORLD,
            entities=["postgresql", "database"],
            explicit_user=True,
        )
        db.commit()
        evaluation = mem_runtime.evaluate(
            db, owner,
            observation={
                "summary": "MIX-agent の主DBは MySQL へ移行した",
                "content": "MIX-agent の主DBは MySQL へ移行した",
                "role": mem_types.ROLE_FACT,
                "scope": mem_types.SCOPE_WORLD,
                "source_kind": mem_types.SOURCE_AGENT,
                "entities": ["mysql", "database"],
            },
        )
        assert any(step["action"] == "dispute" for step in evaluation["plan"])
        mem_runtime.apply_plan(db, owner, evaluation["plan"])
        db.commit()
        conflicts = db.scalars(select(MemoryConflict).where(MemoryConflict.owner_id == owner)).all()
        assert conflicts, "dispute should create a MemoryConflict row"


def test_supersede_chains_old_to_new_and_lifecycle_records_evidence(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        original = mem_tools.remember(
            db, owner,
            content="MIX は毎週金曜にリリースしない",
            role=mem_types.ROLE_PREFERENCE,
            scope=mem_types.SCOPE_USER,
            explicit_user=True,
        )
        new = mem_tools.remember(
            db, owner,
            content="MIX は木曜リリースが基本",
            role=mem_types.ROLE_PREFERENCE,
            scope=mem_types.SCOPE_USER,
            explicit_user=True,
        )
        db.commit()
        original_id = next(item["id"] for item in original["applied"] if item.get("action") == "create")
        new_id = next(item["id"] for item in new["applied"] if item.get("action") == "create")
        mem_runtime.supersede(db, owner, old_id=original_id, new_id=new_id, reason="reversal")
        db.commit()
        old_row = db.get(Memory, original_id)
        assert old_row.lifecycle_state == mem_types.LIFECYCLE_SUPERSEDED
        assert old_row.superseded_by_id == new_id
        mem_runtime.attach_evidence(
            db, owner, new_id,
            kind=mem_types.EVIDENCE_USER_MESSAGE,
            ref="user message 2024-09-12",
            summary="ユーザー発話に基づく更新",
            confidence=0.95,
        )
        db.commit()
        evidence_rows = db.scalars(select(MemoryEvidence).where(MemoryEvidence.memory_id == new_id)).all()
        assert evidence_rows and evidence_rows[0].kind == mem_types.EVIDENCE_USER_MESSAGE


def test_group_for_context_buckets_by_role():
    memories = [
        {"id": "a", "role": mem_types.ROLE_DECISION},
        {"id": "b", "role": mem_types.ROLE_FAILURE},
        {"id": "c", "role": mem_types.ROLE_FACT},
        {"id": "d", "role": mem_types.ROLE_PREFERENCE},
        {"id": "e", "role": mem_types.ROLE_GOAL},
        {"id": "f", "role": mem_types.ROLE_QUESTION},
        {"id": "g", "role": mem_types.ROLE_EXPERIENCE},
        {"id": "h", "role": mem_types.ROLE_UNKNOWN},
    ]
    grouped = mem_runtime.group_for_context(memories)
    assert [row["id"] for row in grouped["decisions"]] == ["a"]
    assert [row["id"] for row in grouped["failures"]] == ["b"]
    assert [row["id"] for row in grouped["facts"]] == ["c"]
    assert [row["id"] for row in grouped["preferences"]] == ["d"]
    assert [row["id"] for row in grouped["goals"]] == ["e"]
    assert [row["id"] for row in grouped["questions"]] == ["f"]
    assert [row["id"] for row in grouped["experiences"]] == ["g"]
    assert [row["id"] for row in grouped["other"]] == ["h"]


def test_recall_debug_exposes_pipeline_weights_and_keywords(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        mem_tools.remember(
            db, owner,
            content="Postgres のマイグレーションはDocker経由で実施する",
            role=mem_types.ROLE_DECISION,
            scope=mem_types.SCOPE_PROJECT,
            role_metadata={
                "decision": "Postgres migration は Docker 経由で実施",
                "reason": "本番と環境を揃えるため",
                "alternatives": ["直接 psql で実行"],
                "rejected_alternatives": [{"option": "psql 直実行", "reason": "本番環境と整合しない"}],
                "source": "user",
                "status": "active",
            },
            explicit_user=True,
        )
        db.commit()
        debug = mem_runtime.recall(
            db, owner,
            {"text": "マイグレーションを実行したい", "kind": "test"},
            debug=True,
        )
        assert "memories" in debug and debug["memories"]
        assert debug["debug"]["weights"]["decision_relevance"] >= 0.0
        assert "lexical" in debug["debug"]
        assert debug["debug"]["considered"] >= 1
