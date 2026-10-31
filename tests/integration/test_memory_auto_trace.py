"""ChatGPT-style retrieval tests: flat search shape."""

from mix_agent.db.models import Settings, User
from mix_agent.db.session import SessionLocal
from mix_agent.memory import service
from sqlalchemy import select


def owner_id(db):
    return db.scalar(select(User.id))


def test_search_debug_shape(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        service.change(db, owner, "MIX Providerの設計", source_run="explicit-user-request")
        service.change(db, owner, "回答は日本語にする", source_run="explicit-user-request")
        db.commit()
        result = service.search(db, owner, "MIX Provider", debug=True)
        assert result["debug"]["stop_reason"] == "flat"
        assert result["debug"]["kept"] <= 8
        assert result["memories"]


def test_settings_user_tunable_keys(signed):
    # Only user-facing keys remain tunable; old graph keys are gone.
    assert signed.put("/api/v1/settings", json={"memory_auto_formation": False}).status_code == 200
    assert signed.put("/api/v1/settings", json={"memory_auto_formation": True}).status_code == 200
    with SessionLocal() as db:
        row = db.scalar(select(Settings).where(Settings.id == "settings", Settings.owner_id == owner_id(db)))
        assert "memory_max_depth" not in (row.data or {})
    assert (
        signed.put("/api/v1/settings", json={"memory_seed_limit": 12, "memory_result_limit": 5}).status_code
        == 200
    )


def test_debug_search_endpoint(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        service.change(db, owner, "MIX Providerの設計", source_run="explicit-user-request")
        db.commit()
    response = signed.get("/api/v1/memories-debug/search", params={"q": "MIX Provider"})
    assert response.status_code == 200
    assert response.json()["memories"]
