from mix_agent.db.models import Memory, MemoryAssociation, User
from mix_agent.db.session import SessionLocal
from mix_agent.memory import service
from sqlalchemy import func, select


def owner_id(db):
    return db.scalar(select(User.id))


def test_latent_trace_reinforces_instead_of_duplicating(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        first = service.change(db, owner, "OSSを優先する", confidence=.55, strength=.4)
        assert first["lifecycle_state"] == "latent"
        for _ in range(6):
            result = service.change(db, owner, "OSSを優先する", confidence=.55, strength=.4)
        db.commit()
        assert result["deduplicated"] is True
        assert result["lifecycle_state"] == "established"
        assert db.scalar(select(func.count()).select_from(Memory).where(Memory.owner_id == owner)) == 1


def test_spreading_activation_is_bounded_and_explained(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        source = service.change(db, owner, "MIX Providerの設計", source_run="explicit-user-request")
        target = service.change(db, owner, "一時障害だけ再試行する", source_run="explicit-user-request")
        db.add(MemoryAssociation(owner_id=owner, source_memory_id=source["id"], target_memory_id=target["id"], weight=.9, confidence=.9, data={"relation": "causal"}))
        db.commit()
        result = service.search(db, owner, "MIX Provider", settings={"max_depth": 1, "max_candidates": 8}, debug=True)
        assert len(result["memories"]) <= 8
        assert target["id"] in {row["id"] for row in result["memories"]}
        assert result["debug"]["association_expansion"]


def test_unrelated_trace_is_not_false_recalled(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        service.change(db, owner, "回答は日本語にする", source_run="explicit-user-request")
        db.commit()
        assert service.search(db, owner, "南極のペンギンの個体数") == []


def test_secret_redaction_and_explicit_forget_parser():
    assert "private-value" not in service.redact_sensitive("api_key=private-value")
    assert service.explicit_forget_candidate("Chromeをメインに使うを忘れて") == "Chromeをメインに使う"


def test_explicit_forget_archives_dependent_trace(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        source = service.change(db, owner, "サービスAは追跡のため却下", source_run="explicit-user-request")
        derived = service.change(db, owner, "サービス選択ではプライバシーを重視", source_run="explicit-user-request", metadata={"source_refs": [source["id"]]}, strength=.4)
        db.add(MemoryAssociation(owner_id=owner, source_memory_id=source["id"], target_memory_id=derived["id"], weight=.8, confidence=.8, data={"relation": "consolidated_from"}))
        db.flush()
        service.change(db, owner, memory_id=source["id"], delete=True)
        db.commit()
        assert db.get(Memory, source["id"]).lifecycle_state == "deleted"
        assert db.get(Memory, derived["id"]).lifecycle_state == "archived"
