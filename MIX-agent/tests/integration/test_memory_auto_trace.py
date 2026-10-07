from mix_agent.db.models import MemoryAssociation, Settings, User
from mix_agent.db.session import SessionLocal
from mix_agent.memory import service
from sqlalchemy import select

AUTOMATIC_KEYS = (
    "memory_max_depth",
    "memory_max_candidates",
    "memory_retrieval_budget_ms",
    "memory_min_association_weight",
    "memory_activation_decay",
)


def owner_id(db):
    return db.scalar(select(User.id))


def linked_graph(db, owner, pairs=1, weight=0.9):
    contents = [
        "MIX Providerの設計",
        "一時障害だけ再試行する",
        "回答は日本語にする",
        "OSSを優先する",
        "プライバシーを重視する",
        "デプロイは金曜を避ける",
        "テストはDockerで回す",
        "ログは構造化する",
    ]
    ids = [service.change(db, owner, content, source_run="explicit-user-request")["id"] for content in contents]
    created = 0
    for source in ids:
        for target in ids:
            if created >= pairs or source == target:
                continue
            db.add(MemoryAssociation(owner_id=owner, source_memory_id=source, target_memory_id=target, weight=weight, confidence=.9, data={"relation": "contextual"}))
            created += 1
    db.flush()
    return ids


def test_auto_config_scales_with_graph(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        empty = service.search(db, owner, "MIX Provider", debug=True)["debug"]["auto"]
        assert empty["max_depth"] == 0
        assert empty["edge_count"] == 0
        assert empty["trace_count"] == 0
        assert empty["retrieval_budget_ms"] == 82

        graph = linked_graph(db, owner, pairs=1)
        db.commit()
        small = service.search(db, owner, "MIX Provider", debug=True)["debug"]["auto"]
        assert small["trace_count"] == len(graph)
        assert small["edge_count"] == 1
        assert small["max_depth"] == 1
        assert 0.10 <= small["min_association_weight"] <= 0.30
        assert 0.40 <= small["activation_decay"] <= 0.70
        assert 60 <= small["retrieval_budget_ms"] <= 400

        for source in graph:
            for target in graph:
                if source == target:
                    continue
                exists = db.scalar(select(MemoryAssociation).where(
                    MemoryAssociation.owner_id == owner,
                    MemoryAssociation.source_memory_id == source,
                    MemoryAssociation.target_memory_id == target,
                ))
                if not exists:
                    db.add(MemoryAssociation(owner_id=owner, source_memory_id=source, target_memory_id=target, weight=.5, confidence=.7, data={"relation": "contextual"}))
        db.commit()
        large = service.search(db, owner, "MIX Provider", debug=True)["debug"]["auto"]
        assert large["edge_count"] >= 40
        assert large["max_depth"] == 2


def test_settings_overrides_are_ignored(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        linked_graph(db, owner, pairs=1)
        db.commit()
        result = service.search(db, owner, "MIX Provider", settings={
            "max_depth": 3,
            "max_candidates": 256,
            "min_association_weight": 0.05,
            "retrieval_budget_ms": 1000,
            "activation_decay": 0.9,
        }, debug=True)
        debug = result["debug"]
        assert debug["ignored_settings"] == ["activation_decay", "max_candidates", "max_depth", "min_association_weight", "retrieval_budget_ms"]
        assert debug["auto"]["max_depth"] == 1
        assert debug["auto"]["max_candidates"] < 256
        assert debug["auto"]["min_association_weight"] >= 0.10
        assert debug["auto"]["retrieval_budget_ms"] <= 400
        assert debug["auto"]["activation_decay"] <= 0.70

        limited = service.search(db, owner, "MIX Provider", settings={"result_limit": 1}, debug=True)
        assert limited["debug"]["honored_settings"] == ["result_limit"]
        assert len(limited["memories"]) == 1


def test_debug_trace_records_stages_and_stop_reason(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        linked_graph(db, owner, pairs=3)
        db.commit()
        result = service.search(db, owner, "MIX Provider", debug=True)
        debug = result["debug"]
        stages = [step["stage"] for step in debug["trace"]]
        assert stages[0] == "seed"
        assert stages[-1] == "rank"
        seed = debug["trace"][0]
        assert seed["found"] >= 1 and "thresholds" in seed
        hop = next(step for step in debug["trace"] if step["stage"] == "hop")
        assert hop["depth"] == 1
        assert hop["edges_considered"] >= 1
        assert hop["edges_below_threshold"] >= 0
        assert hop["accepted"] >= 1
        assert hop["elapsed_ms"] >= 0
        rank = debug["trace"][-1]
        assert rank["kept"] == len(result["memories"]) <= rank["result_limit"]
        assert debug["stop_reason"] in {"depth", "no_frontier", "deadline", "candidate_budget", "no_edges"}
        assert debug["elapsed_ms"] >= 0
        assert debug["association_expansion"]
        assert debug["budget_exhausted"] is False


def test_automatic_memory_settings_are_not_tunable(signed):
    assert signed.put("/api/v1/settings", json={"memory_max_depth": 3}).status_code == 422
    assert signed.put("/api/v1/settings", json={"memory_retrieval_budget_ms": 900}).status_code == 422
    assert signed.put("/api/v1/settings", json={"memory_min_association_weight": 0.05}).status_code == 422
    exposed = signed.get("/api/v1/settings").json()["data"]
    for key in AUTOMATIC_KEYS:
        assert key not in exposed
    with SessionLocal() as db:
        row = db.scalar(select(Settings).where(Settings.id == "settings", Settings.owner_id == owner_id(db)))
        row.data = {**row.data, "memory_max_depth": 3, "memory_activation_decay": 0.9}
        db.commit()
    assert "memory_max_depth" not in signed.get("/api/v1/settings").json()["data"]
    assert signed.put("/api/v1/settings", json={"memory_auto_formation": False}).status_code == 200
    with SessionLocal() as db:
        row = db.scalar(select(Settings).where(Settings.id == "settings", Settings.owner_id == owner_id(db)))
        for key in AUTOMATIC_KEYS:
            assert key not in row.data
    assert signed.put("/api/v1/settings", json={"memory_seed_limit": 12, "memory_result_limit": 5}).status_code == 200


def test_debug_search_endpoint_exposes_auto_and_trace(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        linked_graph(db, owner, pairs=1)
        db.commit()
    response = signed.get("/api/v1/memories-debug/search", params={"q": "MIX Provider"})
    assert response.status_code == 200
    payload = response.json()
    assert payload["debug"]["auto"]["trace_count"] >= 1
    assert payload["debug"]["trace"][0]["stage"] == "seed"
    assert payload["debug"]["stop_reason"]
