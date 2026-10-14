"""Phase 4 of ``docs/memory-spec.md`` §4: `memory_links`, the knowledge graph.

One row = one directed edge. What these tests pin down, in §4's own order:

* the uniqueness rule on ``(owner_id, source_memory_id, target_memory_id,
  kind)`` — idempotent upsert *and* the constraint in PostgreSQL itself;
* §4's "二重管理にしない": `supersedes` mirrors `superseded_by_id` and
  `contradicts` mirrors `disputed_by_id`, in **both** directions every
  transition can move them (supersede, dispute, resolve, revision restore);
* the link *direction* §1/§3 imply — `source` is the id the §3 column names
  (the replacement / the contradicting counterpart), `target` is the row that
  carries the column;
* recall expansion: seed → exactly one hop, capped by degree and count, and
  re-gated on §2's `retrievable_lifecycles` (a pinned row survives, a
  superseded one does not);
* the three generation sources: extraction-time suggestion, coactivation on an
  explicit activating search only, and §5 embedding neighbours behind a
  setting that is **off by default**;
* the §4 endpoint the roadmap asks for (4-c).
"""

from __future__ import annotations

import datetime
from types import SimpleNamespace

import pytest
from mix_agent.db.models import Memory, MemoryLink, MemoryRevision, Settings, User, now
from mix_agent.db.session import SessionLocal
from mix_agent.memory import jobs as mem_jobs
from mix_agent.memory import runtime as mem_runtime
from mix_agent.memory import service as mem_service
from sqlalchemy import func, inspect, select, text
from sqlalchemy.exc import IntegrityError


def owner_id(db):
    return db.scalar(select(User.id))


def _age(db, row_id, days=200):
    """Push a row's recency out of §5.2's scoring window.

    A *fresh* row with no lexical overlap still clears `min_score` through the
    recency term alone (0.15 × ~1.0), so any row that must not be a seed — one
    that may only be reached through a link — has to be aged past the point
    where recency alone stops qualifying it.
    """
    row = db.get(Memory, row_id)
    row.updated_at = now() - datetime.timedelta(days=days)
    db.flush()
    return row


def _edges(db, kind=None, *, source_id=None, target_id=None):
    statement = select(MemoryLink)
    if kind is not None:
        statement = statement.where(MemoryLink.kind == kind)
    if source_id is not None:
        statement = statement.where(MemoryLink.source_memory_id == source_id)
    if target_id is not None:
        statement = statement.where(MemoryLink.target_memory_id == target_id)
    return list(db.scalars(statement))


def _link_count(db):
    return int(db.scalar(select(func.count()).select_from(MemoryLink)) or 0)


def test_the_unique_constraint_holds_at_the_database_and_in_the_upsert(signed):
    """§4: one row per (owner, source, target, kind) — asserted three ways."""
    with SessionLocal() as db:
        owner = owner_id(db)
        first = mem_service.change(db, owner, "一意制約の始点ノード", source_run="explicit-user-request")
        second = mem_service.change(db, owner, "一意制約の終点ノード", source_run="explicit-user-request")
        db.flush()

        # (1) the upsert is idempotent: same tuple, same row, same id.
        opened = mem_service.link(db, owner, second["id"], first["id"], "relates_to")
        again = mem_service.link(db, owner, second["id"], first["id"], "relates_to")
        assert opened["id"] == again["id"]
        assert _link_count(db) == 1
        assert (opened["weight"], opened["coactivation_count"]) == (0.2, 0)

        # (2) migration 0025 put the rule in the table, not only in the code.
        constraints = inspect(db.get_bind()).get_unique_constraints("memory_links")
        assert any(
            set(constraint["column_names"])
            == {"owner_id", "source_memory_id", "target_memory_id", "kind"}
            for constraint in constraints
        )

        # (3) PostgreSQL itself refuses a hand-written duplicate. A savepoint
        # keeps the rest of the fixture's transaction intact afterwards.
        with pytest.raises(IntegrityError), db.begin_nested():
            db.execute(
                text(
                    "INSERT INTO memory_links (id, owner_id, source_memory_id, target_memory_id,"
                    " kind, weight, coactivation_count, created_at, updated_at)"
                    " VALUES ('raw-duplicate', :owner, :source, :target, 'relates_to', 0.2, 0, now(), now())"
                ),
                {"owner": owner, "source": second["id"], "target": first["id"]},
            )
        assert _link_count(db) == 1


def test_supersede_writes_the_column_and_its_mirror_with_the_same_direction(signed):
    """§4 + §3: `kind=supersedes` **is** `superseded_by_id`, source = replacement."""
    with SessionLocal() as db:
        owner = owner_id(db)
        old = mem_service.change(db, owner, "古い説明は五段階で採点する", source_run="explicit-user-request")
        new = mem_service.change(db, owner, "新しい説明は七段階で採点する", source_run="explicit-user-request")
        db.flush()

        mem_runtime.supersede(db, owner, old_id=old["id"], new_id=new["id"], reason="採点方式が変わった")
        db.flush()

        # The §3 column §1 documents as "これを置き換えた Memory" ...
        retired = db.get(Memory, old["id"])
        assert retired.superseded_by_id == new["id"]
        # ... and one edge saying the same thing, in the direction that reads
        # as "source supersedes target": source = 新 id, target = 旧 id.
        edges = _edges(db, "supersedes", target_id=old["id"])
        assert len(edges) == 1
        assert (edges[0].source_memory_id, edges[0].target_memory_id) == (new["id"], old["id"])
        assert edges[0].owner_id == owner

        # §4: no second owner of that state — the mirror cannot drift alone.
        # Clearing the column (an older revision) clears the edge, and
        # restoring it brings the edge back.
        pre_supersede = next(
            item for item in db.scalars(select(MemoryRevision))
            if item.data.get("memory_id") == old["id"]
        )
        mem_service.restore(db, owner, retired, pre_supersede.data["previous"])
        db.flush()
        assert retired.superseded_by_id is None
        assert _edges(db, "supersedes", target_id=old["id"]) == []
        with pytest.raises(ValueError):
            mem_service.link(db, owner, new["id"], old["id"], "nonsense-kind")


def test_dispute_and_resolve_keep_the_edges_in_step_with_both_columns(signed):
    """§4: `kind=contradicts` **is** `disputed_by_id`, and resolve re-points it."""
    with SessionLocal() as db:
        owner = owner_id(db)
        stored = mem_service.change(db, owner, "移動手段は自転車", source_run="explicit-user-request")
        challenge = mem_service.change(db, owner, "移動手段は電車", source_run="explicit-user-request")
        db.flush()

        mem_runtime.dispute(db, owner, memory_id=challenge["id"], conflicting_id=stored["id"], reason="どちらが最新か")
        db.flush()

        row = db.get(Memory, challenge["id"])
        assert row.disputed_by_id == stored["id"]
        # Only the named side is marked (§3), and the edge points the same way:
        # source = §1's "これと矛盾する Memory" (the counterpart), target = the
        # row that carries `disputed_by_id`.
        edges = _edges(db, "contradicts")
        assert len(edges) == 1
        assert (edges[0].source_memory_id, edges[0].target_memory_id) == (stored["id"], challenge["id"])
        assert db.get(Memory, stored["id"]).disputed_by_id is None
        assert _edges(db, "contradicts", source_id=challenge["id"]) == []

        # Settling moves both columns, so it must move both mirrors: the
        # dispute edge disappears (its column was cleared) and the supersede
        # edge appears in its place — same transaction, nothing left over.
        mem_runtime.resolve_conflict(db, owner, conflict_id=challenge["id"], resolution="resolved")
        db.flush()

        assert db.get(Memory, stored["id"]).disputed_by_id is None
        loser = db.get(Memory, challenge["id"])
        assert loser.superseded_by_id == stored["id"]
        assert _edges(db, "contradicts") == []
        superseding = _edges(db, "supersedes")
        assert len(superseding) == 1
        assert (superseding[0].source_memory_id, superseding[0].target_memory_id) == (stored["id"], challenge["id"])
        assert _link_count(db) == 1


def test_revision_restore_follows_the_pointers_it_restores(signed):
    """§4 + §1: restoring a revision moves the column, so the mirror follows."""
    with SessionLocal() as db:
        owner = owner_id(db)
        stored = mem_service.change(db, owner, "コーヒーは毎朝三杯", source_run="explicit-user-request")
        challenge = mem_service.change(db, owner, "コーヒーは毎朝一杯", source_run="explicit-user-request")
        db.flush()

        # Dispute writes the pre-dispute revision itself (§1).
        mem_runtime.dispute(db, owner, memory_id=stored["id"], conflicting_id=challenge["id"])
        db.flush()
        pre_dispute = next(
            item for item in db.scalars(select(MemoryRevision))
            if item.data.get("memory_id") == stored["id"]
        )
        row = db.get(Memory, stored["id"])
        assert row.disputed_by_id == challenge["id"]
        assert len(_edges(db, "contradicts", target_id=stored["id"])) == 1

        # Snapshot the disputed state as well, so both directions can be run.
        disputed = {"id": row.id, **mem_service._trace_data(row)}
        db.add(MemoryRevision(owner_id=owner, data={"memory_id": row.id, "previous": disputed}))
        db.flush()

        # (a) back to before the dispute: the column and the edge both go.
        mem_service.restore(db, owner, row, pre_dispute.data["previous"])
        db.flush()
        assert row.disputed_by_id is None
        assert _edges(db, "contradicts", target_id=stored["id"]) == []

        # (b) forward to the disputed snapshot: the column and the edge both
        # come back — restore does not have to be told about §4 twice.
        mem_service.restore(db, owner, row, disputed)
        db.flush()
        assert row.disputed_by_id == challenge["id"]
        assert [
            (edge.source_memory_id, edge.target_memory_id)
            for edge in _edges(db, "contradicts", target_id=stored["id"])
        ] == [(challenge["id"], stored["id"])]


def test_expansion_follows_exactly_one_hop_and_says_so_in_debug(signed):
    """§4: 「想起時の展開は seed から 1 ホップのみ」 — a second hop never appears."""
    with SessionLocal() as db:
        owner = owner_id(db)
        seed = mem_service.change(db, owner, "起点語を持つ種ノード", source_run="explicit-user-request")
        hop_one = mem_service.change(db, owner, "一歩先の隣接ノード")
        hop_two = mem_service.change(db, owner, "二歩先の遠いノード")
        db.flush()
        mem_service.link(db, owner, seed["id"], hop_one["id"], "relates_to")
        mem_service.link(db, owner, hop_one["id"], hop_two["id"], "relates_to")
        _age(db, hop_one["id"])
        _age(db, hop_two["id"])
        db.flush()

        found = mem_service.search(db, owner, "起点語", debug=True)
        ids = [item["id"] for item in found["memories"]]
        debug = found["debug"]

        assert ids[0] == seed["id"]  # the seed stays the answer, not a link
        assert hop_one["id"] in ids
        assert hop_two["id"] not in ids, "a linked row's own neighbour is two hops away"
        assert next(item for item in found["memories"] if item["id"] == hop_one["id"])["selection_reason"] == "リンクから"
        # TraceView's key keeps its shape: {source, target, activation}.
        assert debug["association_expansion"] == [
            {"source": seed["id"], "target": hop_one["id"], "activation": 0.2, "kind": "relates_to"}
        ]
        assert debug["association_expansion_count"] == 1
        assert debug["association_cutoff"] is None
        assert debug["stop_reason"] == "depth"
        assert debug["seed_ids"] == [seed["id"]]


def test_the_degree_cap_stops_a_walk_before_the_expansion_cap_would(signed):
    """§4: 度数上限 is a constant (5 per seed) and it is reported when it bites."""
    with SessionLocal() as db:
        owner = owner_id(db)
        seed = mem_service.change(db, owner, "高次数の種ノード", source_run="explicit-user-request")
        # The query words live only in the seed: a neighbour that shared them
        # would be a *seed* by §5.2 and could never prove the degree cap.
        neighbours = [
            mem_service.change(db, owner, f"近傍その{index} は素数を数える")
            for index in range(20)
        ]
        db.flush()
        for neighbour in neighbours:
            mem_service.link(db, owner, seed["id"], neighbour["id"], "relates_to")
        for neighbour in neighbours:
            _age(db, neighbour["id"])
        db.flush()

        found = mem_service.search(db, owner, "高次数", debug=True)
        debug = found["debug"]
        added = [item for item in found["memories"] if item["selection_reason"] == "リンクから"]

        assert mem_service.LINK_DEGREE_LIMIT == 5
        assert len(added) == mem_service.LINK_DEGREE_LIMIT
        assert len(found["memories"]) == 1 + mem_service.LINK_DEGREE_LIMIT
        assert debug["association_expansion_count"] == mem_service.LINK_DEGREE_LIMIT
        assert debug["association_cutoff"] == "degree_limit"
        assert {edge["source"] for edge in debug["association_expansion"]} == {seed["id"]}
        assert {edge["target"] for edge in debug["association_expansion"]} <= {
            neighbour["id"] for neighbour in neighbours
        }


def test_expansion_obeys_the_lifecycle_gate_and_keeps_a_pinned_row(signed):
    """§4's expansion is re-gated: hidden rows stay hidden, pinned stays findable."""
    with SessionLocal() as db:
        owner = owner_id(db)
        seed = mem_service.change(db, owner, "寿命判定の種ノード", source_run="explicit-user-request")
        # Again: none of these may share the query's words (§5.2 would make
        # them seeds instead of rows reached through a link).
        active = mem_service.change(db, owner, "稼働する評価ノード")
        retired = mem_service.change(db, owner, "廃止された評価ノード")
        archived = mem_service.change(db, owner, "保存庫の評価ノード")
        pinned = mem_service.change(db, owner, "固定された評価ノード", pinned=True)
        db.flush()

        # One retired through §3's own transition, one by hand (§3's `manual`),
        # one pinned — §3 exempts pinned rows from every automatic retirement.
        mem_runtime.supersede(db, owner, old_id=retired["id"], new_id=active["id"])
        db.get(Memory, archived["id"]).lifecycle_state = "archived"
        db.get(Memory, pinned["id"]).lifecycle_state = "archived"
        db.flush()

        for row_id in (active["id"], retired["id"], archived["id"], pinned["id"]):
            mem_service.link(db, owner, seed["id"], row_id, "relates_to")
        for row_id in (active["id"], retired["id"], archived["id"], pinned["id"]):
            _age(db, row_id)
        db.flush()

        found = mem_service.search(db, owner, "寿命判定", debug=True)
        ids = {item["id"] for item in found["memories"]}

        assert seed["id"] in ids
        assert active["id"] in ids  # ordinary retrievable row
        assert pinned["id"] in ids, "§3: pinned rows are retrievable whatever the lifecycle says"
        assert retired["id"] not in ids, "a link must not resurrect a superseded row"
        assert archived["id"] not in ids, "a link must not resurrect an archived row"
        assert found["debug"]["association_expansion_count"] == 2


def test_coactivation_links_count_up_on_an_activating_search_only(signed):
    """§4 source (2) fires on `activate=True`; context injection never co-creates."""
    with SessionLocal() as db:
        owner = owner_id(db)
        first = mem_service.change(db, owner, "koherence の説明は一つ目", source_run="explicit-user-request")
        second = mem_service.change(db, owner, "koherence の説明は二つ目", source_run="explicit-user-request")
        db.flush()
        assert _link_count(db) == 0

        mem_service.search(db, owner, "koherence", activate=True)
        db.flush()
        pair = _edges(db, "relates_to")
        assert len(pair) == 1
        assert pair[0].coactivation_count == 1
        assert {pair[0].source_memory_id, pair[0].target_memory_id} == {first["id"], second["id"]}

        # The same explicit search again increments, still one row (§4 制約).
        mem_service.search(db, owner, "koherence", activate=True)
        db.flush()
        assert _link_count(db) == 1
        assert _edges(db, "relates_to")[0].coactivation_count == 2

        # §3: context injection searches every turn without activating — it
        # must neither reinforce nor invent links from "mentioned today".
        before = _link_count(db)
        views = mem_runtime.recall(db, owner, {"text": "koherence", "kind": "test"})
        db.flush()
        assert _link_count(db) == before
        assert _edges(db, "relates_to")[0].coactivation_count == 2
        # §5.3: the rendered hits can say how many edges they have (here: 1).
        assert len(views) == 2
        assert {view["link_count"] for view in views} == {1}


def test_link_rejects_a_kind_outside_section_2(signed):
    """§4: `kind` is §2 `relation_kinds` — anything else is a ValueError."""
    with SessionLocal() as db:
        owner = owner_id(db)
        first = mem_service.change(db, owner, "語彙検査の始点", source_run="explicit-user-request")
        second = mem_service.change(db, owner, "語彙検査の終点", source_run="explicit-user-request")
        db.flush()

        for bad in ("friends", "SUPERSEDES", "", None):
            with pytest.raises(ValueError):
                mem_service.link(db, owner, second["id"], first["id"], bad)
        # Valid §2 kinds are accepted, and both endpoints must belong to us.
        assert mem_service.link(db, owner, second["id"], first["id"], "supports")["kind"] == "supports"
        other_owner = db.scalar(select(User.id).where(User.id != owner))
        if other_owner:
            with pytest.raises(ValueError):
                mem_service.link(db, other_owner, second["id"], first["id"], "relates_to")
        assert _link_count(db) == 1


def test_link_refuses_the_two_mirror_kinds_and_leaves_section_3_untouched(signed):
    """§4: 「§3 の列を書きうるのは §3 の遷移だけ」 — `link()` makes no exception.

    Refusing is not enough on its own: the reason §4 forbids the round trip is
    the row it would leave behind, so this also pins that *nothing* moved —
    no pointer, no `lifecycle_state`, no edge.
    """
    with SessionLocal() as db:
        owner = owner_id(db)
        first = mem_service.change(db, owner, "拒否検査の始点ノード", source_run="explicit-user-request")
        second = mem_service.change(db, owner, "拒否検査の終点ノード", source_run="explicit-user-request")
        db.flush()

        before = {
            row_id: (
                db.get(Memory, row_id).superseded_by_id,
                db.get(Memory, row_id).disputed_by_id,
                db.get(Memory, row_id).lifecycle_state,
            )
            for row_id in (first["id"], second["id"])
        }
        assert _link_count(db) == 0

        # Both directions, both mirror kinds: §4 refuses them although §2
        # still lists them as `relation_kinds`.
        for source_id, target_id in ((second["id"], first["id"]), (first["id"], second["id"])):
            for kind in ("supersedes", "contradicts"):
                with pytest.raises(ValueError):
                    mem_service.link(db, owner, source_id, target_id, kind)

        # The §2 check still comes first: an unknown word is reported as one.
        with pytest.raises(ValueError) as unknown:
            mem_service.link(db, owner, second["id"], first["id"], "nonsense-kind")
        assert "unknown relation kind" in str(unknown.value)

        for row_id, state in before.items():
            row = db.get(Memory, row_id)
            assert (row.superseded_by_id, row.disputed_by_id, row.lifecycle_state) == state
            # The row this contract protects: a pointer without the §3 state
            # would still be in `retrievable_lifecycles`.
            assert mem_service.retrievable(row) is True
        assert _link_count(db) == 0


def test_the_associations_endpoint_returns_the_real_edges(signed):
    """§4 endpoint (§9 #3): both directions, owner-scoped, §4's own shape."""
    with SessionLocal() as db:
        owner = owner_id(db)
        old = mem_service.change(db, owner, "API経由の旧ノード", source_run="explicit-user-request")
        new = mem_service.change(db, owner, "API経由の新ノード", source_run="explicit-user-request")
        plain = mem_service.change(db, owner, "API経由のリンク無し行", source_run="explicit-user-request")
        db.flush()
        # The only edge the endpoint may show for a mirror kind is one §3
        # really made: spec §4 has link() refuse `supersedes`, so `supersede`
        # is how this one is created.
        mem_runtime.supersede(db, owner, old_id=old["id"], new_id=new["id"], reason="API 検査のため")
        db.flush()
        assert db.get(Memory, old["id"]).superseded_by_id == new["id"]
        db.commit()

        by_target = signed.get(f"/api/v1/memories/{old['id']}/associations")
        assert by_target.status_code == 200
        edges = by_target.json()
        assert len(edges) == 1
        assert (
            edges[0]["source_memory_id"],
            edges[0]["target_memory_id"],
            edges[0]["kind"],
        ) == (new["id"], old["id"], "supersedes")
        assert (edges[0]["weight"], edges[0]["coactivation_count"]) == (0.2, 0)

        # The other endpoint of the same edge sees it too (both directions).
        by_source = signed.get(f"/api/v1/memories/{new['id']}/associations").json()
        assert [edge["id"] for edge in by_source] == [edges[0]["id"]]

        # §9 #12: the list reports the edge count too, and stays silent for a
        # row with none (the key is additive, exactly like `views.render`).
        listed = {row["id"]: row["data"] for row in signed.get("/api/v1/memories").json()}
        assert listed[old["id"]]["link_count"] == 1
        assert listed[new["id"]]["link_count"] == 1
        assert "link_count" not in listed[plain["id"]]

        # A memory nobody can find has no associations either.
        assert signed.get("/api/v1/memories/no-such-memory/associations").status_code in (403, 404)


def test_extraction_proposes_links_to_the_rows_it_agrees_with(signed):
    """§4 source (1): the extractor's proposal, derived instead of prompted."""
    with SessionLocal() as db:
        owner = owner_id(db)
        stored = mem_service.change(
            db, owner, "formal_language_source では要約を省く", source_run="explicit-user-request",
        )
        db.flush()

        job = SimpleNamespace(owner_id=owner, run_id=None)
        touched = mem_jobs._apply(db, job, [{
            "content": "formal_language_source では箇条書きを三行に収める",
            "action": "CREATE",
            "role": "preference",
        }])
        db.flush()

        assert len(touched) == 1 and touched[0] != stored["id"]
        edge = _edges(db, "relates_to", source_id=touched[0])
        assert [other.target_memory_id for other in edge] == [stored["id"]]
        # §4 制約: suggesting the same thing twice keeps one row.
        assert mem_service.suggest_links(db, owner, touched) == 0
        assert _link_count(db) == 1


def test_embedding_neighbours_stay_off_unless_the_setting_turns_them_on(signed):
    """§4 source (3): 「(3) は既定オフ」 — the gate refuses before the query runs."""
    with SessionLocal() as db:
        owner = owner_id(db)
        first = mem_service.change(db, owner, "埋め込み近傍の基準ノード", source_run="explicit-user-request")
        second = mem_service.change(db, owner, "埋め込み近傍の探索ノード", source_run="explicit-user-request")
        db.flush()
        for row_id in (first["id"], second["id"]):
            row = db.get(Memory, row_id)
            # §5 stores a real embedding; row_vector() refuses anything shorter
            # than 8 values, so a stub has to be that long to be a vector at all.
            row.data = {**(row.data or {}), "embedding": [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]}
        db.flush()

        settings = db.get(Settings, "settings")
        assert settings is not None and settings.owner_id == owner
        assert settings.data.get("memory_link_embedding_neighbors") is not True
        # Off by default: the helper is asked directly and still refuses.
        assert mem_jobs._embedding_neighbor_links_if_enabled(db, owner, [second["id"]]) == 0
        assert _link_count(db) == 0

        settings.data = {**(settings.data or {}), "memory_link_embedding_neighbors": True}
        db.flush()
        assert mem_jobs._embedding_neighbor_links_if_enabled(db, owner, [second["id"]]) == 1
        edge = _edges(db, "relates_to", source_id=second["id"])
        assert [other.target_memory_id for other in edge] == [first["id"]]
        assert edge[0].weight >= mem_service.LINK_NEIGHBOR_MIN_SIMILARITY
