"""Phase 3 of ``docs/memory-spec.md``: integration / conflict / lifetime.

Migration 0024 restores spec §1's eight Memory columns; §3's transitions —
supersede / dispute / resolve / decay / reinforce — are what keep them honest.
Each test pins one promise this phase makes, including the two deliberate
behaviour changes: ``forget`` is still a hard delete (a §3 supersede needs a
replacement id), and a challenger never wins a dispute by default.
"""

from __future__ import annotations

import datetime
import json
from pathlib import Path

import pytest
from alembic import command
from alembic.config import Config as AlembicConfig
from sqlalchemy import create_engine, inspect, select, text
from sqlalchemy.orm import sessionmaker
from sqlalchemy.pool import NullPool

from mix_agent.db import session as db_session
from mix_agent.db.models import Memory, MemoryActionEvent, MemoryRevision, User, now
from mix_agent.db.session import SessionLocal
from mix_agent.memory import jobs as mem_jobs
from mix_agent.memory import runtime as mem_runtime
from mix_agent.memory import service as mem_service
from mix_agent.memory import tools as mem_tools
from mix_agent.memory import views as mem_views

# ``mix_agent/db/session.py`` -> ``<apps/server>/mix_agent/db/session.py``;
# the same anchor resolves to ``/app`` inside the test image, where alembic.ini
# and migrations/ live next to it.
SERVER_DIR = Path(db_session.__file__).resolve().parents[2]
SCRATCH_DATABASE = "mix_lifecycle_scratch"
SCRATCH_OWNER = "owner-0024"

# Spec §1's eight logical columns, restored by migration 0024.
EIGHT = (
    "lifecycle_state",
    "superseded_by_id",
    "disputed_by_id",
    "confidence",
    "salience",
    "role",
    "scope",
    "source_kind",
)

# Three rows exactly as 0022 stored them: flat ``content`` + ``data``, no §1 columns.
SCRATCH_ROWS = (
    ("m-explicit", "Scratchpad の設定を保存する", {"source": "user", "role": "preference"}),
    ("m-auto", "Scratchpad を自動で整理する", {"source": "auto"}),
    ("m-pinned", "Scratchpad を固定する", {"source": "user", "pinned": True}),
)

INSERT_FLAT_ROW = text(
    "INSERT INTO memories (id, owner_id, content, data, created_at, updated_at) "
    "VALUES (:id, :owner, :content, CAST(:data AS jsonb), now(), now())"
)


def owner_id(db):
    return db.scalar(select(User.id))


def _read_flat_rows(engine):
    """Every scratch row as a plain dict, keyed by id, for whole-row comparison."""
    with engine.connect() as conn:
        rows = conn.execute(
            text("SELECT * FROM memories WHERE owner_id = :owner ORDER BY id"),
            {"owner": SCRATCH_OWNER},
        )
        return {row["id"]: dict(row) for row in rows.mappings()}


@pytest.fixture
def scratch_database(monkeypatch):
    """A throw-away PostgreSQL database, with alembic pointed at it.

    ``migrations/env.py`` binds to ``mix_agent.db.session.engine``, so patching
    that one name migrates the scratch database while the suite's real one —
    and every other test — stays untouched. Dropped again on teardown.
    """
    admin = create_engine(
        db_session.engine.url.set(database="postgres"),
        isolation_level="AUTOCOMMIT",
        poolclass=NullPool,
    )
    scratch = create_engine(
        db_session.engine.url.set(database=SCRATCH_DATABASE),
        poolclass=NullPool,
    )
    drop = text(f'DROP DATABASE IF EXISTS "{SCRATCH_DATABASE}" WITH (FORCE)')
    with admin.connect() as conn:
        conn.execute(drop)
        conn.execute(text(f'CREATE DATABASE "{SCRATCH_DATABASE}"'))
    monkeypatch.setattr(db_session, "engine", scratch)
    try:
        yield scratch
    finally:
        scratch.dispose()
        with admin.connect() as conn:
            conn.execute(drop)
        admin.dispose()


def test_migration_0024_restores_the_eight_columns_and_is_rerunnable(scratch_database):
    cfg = AlembicConfig(str(SERVER_DIR / "alembic.ini"))
    cfg.set_main_option("script_location", str(SERVER_DIR / "migrations"))

    command.upgrade(cfg, "0022")
    flat_columns = {column["name"] for column in inspect(scratch_database).get_columns("memories")}
    assert not flat_columns & set(EIGHT), "0022 flattened Memory: the eight §1 columns must be gone"

    with scratch_database.begin() as conn:
        for row_id, content, data in SCRATCH_ROWS:
            conn.execute(
                INSERT_FLAT_ROW,
                {
                    "id": row_id,
                    "owner": SCRATCH_OWNER,
                    "content": content,
                    "data": json.dumps({**data, "content": content}, ensure_ascii=False),
                },
            )

    command.upgrade(cfg, "0024")
    first = _read_flat_rows(scratch_database)
    explicit, automatic, pinned = (first[row_id] for row_id in ("m-explicit", "m-auto", "m-pinned"))

    # data.role (§6's extractor) wins over the default, data.source is derived.
    assert explicit["lifecycle_state"] == "established"
    assert explicit["role"] == "preference"
    assert explicit["scope"] == "user"
    assert explicit["source_kind"] == "user"
    assert explicit["confidence"] == 0.7
    assert explicit["salience"] == 0.5
    assert explicit["superseded_by_id"] is None and explicit["disputed_by_id"] is None
    assert automatic["lifecycle_state"] == "established"
    assert automatic["role"] == "fact"
    assert automatic["source_kind"] == "agent"  # 'auto' is not a §2 source
    assert automatic["confidence"] == 0.7 and automatic["salience"] == 0.5
    assert pinned["salience"] == 1.0  # §3 exempts pinned rows from decay
    assert pinned["role"] == "fact" and pinned["source_kind"] == "user"

    # Re-runnable: a second run over the same rows must change nothing.
    with scratch_database.begin() as conn:
        conn.execute(text("UPDATE alembic_version SET version_num = '0022'"))
    command.upgrade(cfg, "0024")
    assert _read_flat_rows(scratch_database) == first

    # §3's retrieval gate reads the restored column: superseded leaves recall,
    # pinned stays findable even once archived, and review shows everything.
    with scratch_database.begin() as conn:
        conn.execute(text(
            "UPDATE memories SET lifecycle_state = 'superseded', superseded_by_id = 'm-explicit' "
            "WHERE id = 'm-auto'"
        ))
        conn.execute(text("UPDATE memories SET lifecycle_state = 'archived' WHERE id = 'm-pinned'"))

    with sessionmaker(scratch_database)() as db:
        found = mem_service.search(db, SCRATCH_OWNER, "Scratchpad", debug=True)
        review = mem_service.list_traces(db, SCRATCH_OWNER)

    assert {row["id"] for row in found["memories"]} == {"m-explicit", "m-pinned"}
    assert found["debug"]["considered"] == 2
    assert found["debug"]["lifecycle_excluded"] == 1
    assert {row["id"] for row in review} == {"m-explicit", "m-auto", "m-pinned"}


def test_supersede_keeps_the_row_writes_a_revision_and_hides_it_from_recall(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        old = mem_service.change(db, owner, "古い設定は英語で保存する", source_run="explicit-user-request")
        new = mem_service.change(db, owner, "新しい設定は日本語で保存する")
        db.flush()
        assert old["lifecycle_state"] == "established"

        result = mem_runtime.supersede(db, owner, old_id=old["id"], new_id=new["id"], reason="修正版を採用")
        db.flush()

        assert result["lifecycle"] == "superseded"
        row = db.get(Memory, old["id"])
        assert row is not None  # §3: a supersede is never a delete
        assert row.lifecycle_state == "superseded"
        assert row.superseded_by_id == new["id"]

        # §1: the pre-transition state is on disk before the transition.
        revisions = [item for item in db.scalars(select(MemoryRevision)) if item.data.get("memory_id") == old["id"]]
        assert len(revisions) == 1
        assert revisions[0].data["previous"]["lifecycle_state"] == "established"

        # Out of recall, still in review.
        hits = {item["id"] for item in mem_service.search(db, owner, "設定")}
        assert old["id"] not in hits and new["id"] in hits
        assert old["id"] in {item["id"] for item in mem_service.list_traces(db, owner)}

        events = [item for item in db.scalars(select(MemoryActionEvent)) if item.data.get("action") == "SUPERSEDE"]
        assert len(events) == 1
        assert events[0].data["related_memory_id"] == new["id"]

        # A supersede needs a replacement (forget ≠ supersede: forget is
        # service.change(..., delete=True), which keeps its own revision).
        with pytest.raises(ValueError):
            mem_runtime.supersede(db, owner, old_id=new["id"], new_id=None)
        with pytest.raises(ValueError):
            mem_runtime.supersede(db, owner, old_id=new["id"], new_id=new["id"])


def test_dispute_marks_one_side_and_resolve_settles_the_pair(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        stored = mem_service.change(db, owner, "東京に三年暮らしている", source_run="explicit-user-request")
        challenge = mem_service.change(db, owner, "大阪に五年暮らしている")
        db.flush()

        marked = mem_runtime.dispute(db, owner, memory_id=challenge["id"], conflicting_id=stored["id"], reason="矛盾を検出")
        db.flush()

        # Only the named row is marked: an open conflict cannot hide the stored fact.
        row, other = db.get(Memory, challenge["id"]), db.get(Memory, stored["id"])
        assert marked["lifecycle"] == "disputed"
        assert (row.lifecycle_state, row.disputed_by_id) == ("disputed", stored["id"])
        assert (other.lifecycle_state, other.disputed_by_id) == ("established", None)
        pair = {item["id"] for item in mem_service.search(db, owner, "暮らしている")}
        assert stored["id"] in pair and challenge["id"] not in pair
        dispute_revisions = [item for item in db.scalars(select(MemoryRevision)) if item.data.get("memory_id") == challenge["id"]]
        assert len(dispute_revisions) == 1
        assert dispute_revisions[0].data["previous"]["lifecycle_state"] == "candidate"

        resolved = mem_runtime.resolve_conflict(
            db, owner, conflict_id=challenge["id"], resolution="resolved", prefer=challenge["id"],
        )
        db.flush()

        assert resolved["winner"] == challenge["id"] and resolved["loser"] == stored["id"]
        assert row.lifecycle_state == "active" and row.disputed_by_id is None
        assert other.lifecycle_state == "superseded"
        assert other.superseded_by_id == challenge["id"] and other.disputed_by_id is None
        # Both sides kept their pre-resolution state (§1: no transition loses history).
        revisions = list(db.scalars(select(MemoryRevision)))
        assert len(revisions) == 3
        pair = {item["id"] for item in mem_service.search(db, owner, "暮らしている")}
        assert pair == {challenge["id"]}

        events = [item for item in db.scalars(select(MemoryActionEvent)) if item.data.get("action") == "RESOLVE"]
        assert len(events) == 1
        assert events[0].data["memory_id"] == stored["id"]  # the rejected side
        assert events[0].data["related_memory_id"] == challenge["id"]  # the adopted side


def test_resolve_without_a_preference_does_not_let_the_challenger_win(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        stored = mem_service.change(db, owner, "休日は土曜の朝に走る", source_run="explicit-user-request")
        challenge = mem_service.change(db, owner, "休日は日曜の夕方に走る")
        db.flush()
        mem_runtime.dispute(db, owner, memory_id=challenge["id"], conflicting_id=stored["id"])
        db.flush()

        with pytest.raises(ValueError):
            mem_runtime.resolve_conflict(db, owner, conflict_id=challenge["id"], resolution="resolved", prefer="someone-else")

        resolved = mem_runtime.resolve_conflict(db, owner, conflict_id=challenge["id"], resolution="resolved")
        db.flush()

        assert resolved["winner"] == stored["id"]  # the memory already stored is the default winner
        assert db.get(Memory, stored["id"]).lifecycle_state == "active"
        assert db.get(Memory, challenge["id"]).lifecycle_state == "superseded"


def test_dismissing_a_conflict_expires_the_rejected_side_without_deleting_it(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        stored = mem_service.change(db, owner, "好きな映画は時代劇", source_run="explicit-user-request")
        challenge = mem_service.change(db, owner, "好きな映画はアニメ")
        db.flush()
        mem_runtime.dispute(db, owner, memory_id=challenge["id"], conflicting_id=stored["id"])
        db.flush()

        resolved = mem_runtime.resolve_conflict(db, owner, conflict_id=challenge["id"], resolution="dismiss")
        db.flush()

        rejected = db.get(Memory, challenge["id"])
        assert resolved["loser_state"] == "expired"
        assert rejected is not None and rejected.lifecycle_state == "expired"
        assert rejected.superseded_by_id == stored["id"]
        assert {item["id"] for item in mem_service.search(db, owner, "映画")} == {stored["id"]}
        assert challenge["id"] in {item["id"] for item in mem_service.list_traces(db, owner)}


def test_resolve_route_settles_what_the_list_shows(signed):
    """§9 #12 / §3: a dispute the UI can *see* is one the UI can settle.

    The route is a thin HTTP face over §3's ``resolve_conflict`` — the point
    of this test is that the two §3 contracts survive the round trip: the
    side ``disputed_by_id`` names wins when the client sends no preference
    (the UI must not pre-select one), and both rows keep a revision.
    """
    with SessionLocal() as db:
        owner = owner_id(db)
        stored = mem_service.change(db, owner, "走るのは土曜の朝", source_run="explicit-user-request")
        challenge = mem_service.change(db, owner, "走るのは日曜の夕方")
        db.flush()
        mem_runtime.dispute(db, owner, memory_id=challenge["id"], conflicting_id=stored["id"])
        db.commit()

    listed = {row["id"]: row for row in signed.get("/api/v1/memories").json()}
    assert listed[challenge["id"]]["data"]["lifecycle_state"] == "disputed"
    assert listed[challenge["id"]]["data"]["disputed_by_id"] == stored["id"]

    # Sent exactly as the UI sends its default action: no `prefer` at all.
    settled = signed.post(f"/api/v1/memories/{challenge['id']}/resolve", json={"resolution": "resolved"})
    assert settled.status_code == 200
    body = settled.json()
    assert body["decision"] == "resolved"
    assert body["id"] == stored["id"]  # the already-stored side, §3's default winner
    assert body["loser_id"] == challenge["id"] and body["loser_state"] == "superseded"

    with SessionLocal() as db:
        winner = db.get(Memory, stored["id"])
        loser = db.get(Memory, challenge["id"])
        assert winner.lifecycle_state == "active" and winner.disputed_by_id is None
        assert loser.lifecycle_state == "superseded" and loser.superseded_by_id == stored["id"]
        revisions = [
            item for item in db.scalars(select(MemoryRevision))
            if item.data.get("memory_id") in (stored["id"], challenge["id"])
        ]
        # 1 revision the dispute wrote + 2 the settlement wrote: §3 keeps a
        # pre-resolution snapshot of *both* rows, not just the rejected one.
        assert len(revisions) == 3
        states = {(item.data["memory_id"], item.data["previous"].get("lifecycle_state")) for item in revisions}
        assert (stored["id"], "established") in states  # the adopted side, as it was
        assert (challenge["id"], "disputed") in states  # the rejected side, as it was

    # A row with nothing to settle is rejected, and an unknown id is a 404.
    assert signed.post(f"/api/v1/memories/{stored['id']}/resolve", json={}).status_code == 422
    assert signed.post("/api/v1/memories/no-such-memory/resolve", json={}).status_code == 404


def test_resolve_route_lets_the_client_choose_the_winner_or_dismiss(signed):
    """The same route also carries §3's explicit choice and its ``dismiss``."""
    with SessionLocal() as db:
        owner = owner_id(db)
        stored = mem_service.change(db, owner, "公開は月曜の午前", source_run="explicit-user-request")
        challenge = mem_service.change(db, owner, "公開は金曜の午後")
        db.flush()
        mem_runtime.dispute(db, owner, memory_id=challenge["id"], conflicting_id=stored["id"])
        db.commit()

    chosen = signed.post(
        f"/api/v1/memories/{challenge['id']}/resolve",
        json={"resolution": "resolved", "prefer": challenge["id"]},
    )
    assert chosen.status_code == 200
    assert chosen.json()["id"] == challenge["id"]
    with SessionLocal() as db:
        assert db.get(Memory, challenge["id"]).lifecycle_state == "active"
        assert db.get(Memory, stored["id"]).lifecycle_state == "superseded"

    # `dismiss` follows the pair instead of keeping one side: loser expires.
    with SessionLocal() as db:
        owner = owner_id(db)
        kept = mem_service.change(db, owner, "持ち合わせは千円")
        rival = mem_service.change(db, owner, "持ち合わせは五百円")
        db.flush()
        mem_runtime.dispute(db, owner, memory_id=rival["id"], conflicting_id=kept["id"])
        db.commit()
    dismissed = signed.post(f"/api/v1/memories/{rival['id']}/resolve", json={"resolution": "dismiss"})
    assert dismissed.status_code == 200
    assert dismissed.json()["loser_state"] == "expired"


def test_the_list_filter_actually_filters_and_refuses_an_unknown_state(signed):
    """§9 #14: ``state`` used to be accepted and then ignored.

    A filter that does nothing is worse than no filter at all — the caller
    cannot tell "nothing matched" from "I filtered wrong", so the list simply
    looked empty. It now narrows exactly, and a value outside §2 is a 422
    rather than an empty page.
    """
    with SessionLocal() as db:
        owner = owner_id(db)
        stored = mem_service.change(db, owner, "築地の朝市は火曜だけ", source_run="explicit-user-request")
        challenge = mem_service.change(db, owner, "築地の朝市は金曜だけ")
        db.flush()
        mem_runtime.dispute(db, owner, memory_id=challenge["id"], conflicting_id=stored["id"])
        db.commit()

    def ids(state=None):
        response = signed.get("/api/v1/memories" + (f"?state={state}" if state else ""))
        assert response.status_code == 200
        return {row["id"] for row in response.json()}

    everything = ids()
    assert {stored["id"], challenge["id"]} <= everything

    # It narrows, and the row it keeps is the one §3 put in that state.
    disputed = ids("disputed")
    assert challenge["id"] in disputed and stored["id"] not in disputed

    # `pinned` is §3's exemption from *retrieval*, not from a filter: asking
    # for `archived` means the archived rows, pinned or not.
    with SessionLocal() as db:
        row = db.get(Memory, challenge["id"])
        row.lifecycle_state = "archived"
        row.data = {**(row.data or {}), "pinned": True}
        db.commit()
    assert challenge["id"] in ids("archived")
    assert challenge["id"] not in ids("disputed")

    # A typo is refused instead of being answered with an empty page.
    assert signed.get("/api/v1/memories?state=nonsense").status_code == 422


def test_daily_decay_weakens_idle_rows_once_and_archives_at_the_floor(signed, monkeypatch):
    monkeypatch.setattr(mem_jobs, "_LAST_DECAY_DATE", datetime.date(2000, 1, 1))
    with SessionLocal() as db:
        owner = owner_id(db)
        idle = mem_service.change(db, owner, "忘れかけの事実")
        ancient = mem_service.change(db, owner, "ずっと忘れられた事実")
        pinned = mem_service.change(db, owner, "固定された事実", pinned=True)
        fresh = mem_service.change(db, owner, "今の事実")
        db.flush()
        for row_id, days in ((idle["id"], 30), (ancient["id"], 500), (pinned["id"], 500)):
            db.get(Memory, row_id).updated_at = now() - datetime.timedelta(days=days)
        db.flush()
        before = {row_id: db.get(Memory, row_id).updated_at for row_id in (idle["id"], ancient["id"], pinned["id"], fresh["id"])}

        assert mem_jobs.daily_decay(db) == 2  # idle + ancient; pinned and fresh are skipped

        idle_row, ancient_row = db.get(Memory, idle["id"]), db.get(Memory, ancient["id"])
        pinned_row, fresh_row = db.get(Memory, pinned["id"]), db.get(Memory, fresh["id"])
        assert 0.1 < idle_row.data["strength"] < 0.6  # weakened, but not retired
        assert idle_row.lifecycle_state != "archived"
        assert ancient_row.data["strength"] <= 0.1
        assert ancient_row.lifecycle_state == "archived"  # §3: at the floor it stops being retrievable
        assert "strength" not in pinned_row.data and pinned_row.salience == 1.0  # §3 exempts pinned rows
        assert "strength" not in fresh_row.data  # < DECAY_MIN_DAYS is left alone

        # updated_at is the *input* to decay, so decay never rewrites it.
        assert {row_id: db.get(Memory, row_id).updated_at for row_id in before} == before
        # Scalar decay recomputes §1's own fields: no revision, one DECAY event.
        assert list(db.scalars(select(MemoryRevision))) == []
        decay_events = [item for item in db.scalars(select(MemoryActionEvent)) if item.data.get("action") == "DECAY"]
        assert [item.data["memory_id"] for item in decay_events] == [ancient["id"]]

        # The daily marker: a second pass today does nothing.
        assert mem_jobs.daily_decay(db) == 0


def test_reinforce_moves_recency_and_the_strength_fields(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        created = mem_service.change(db, owner, "会議は火曜の午後に決める", source_run="explicit-user-request")
        row = db.get(Memory, created["id"])
        stale = now() - datetime.timedelta(days=10)
        row.updated_at = stale
        db.flush()

        steps = mem_runtime.apply_plan(
            db, owner, [{"action": "reinforce", "memory_id": row.id, "reason": "duplicate"}],
        )
        db.flush()

        assert [step["action"] for step in steps] == ["reinforce"]
        assert row.updated_at > stale
        assert row.salience == pytest.approx(0.55)
        assert row.confidence == pytest.approx(0.72)
        assert row.data["strength"] == pytest.approx(0.65)
        assert row.data["activation_count"] == 1

        # §1 reactivation: an explicit search hit reinforces too.
        hits = {item["id"] for item in mem_service.search(db, owner, "火曜 午後", activate=True)}
        assert row.id in hits
        assert row.data["strength"] == pytest.approx(0.70)
        assert row.data["activation_count"] == 2
        assert row.salience == pytest.approx(0.60)

        # Context injection must not rewrite recency behind the user's back.
        mem_runtime.recall(db, owner, {"text": "火曜 午後", "kind": "test"})
        assert row.data["activation_count"] == 2


def test_recall_only_sees_retrievable_lifecycles(signed):
    states = {
        "m-established": ("established", False),
        "m-superseded": ("superseded", False),
        "m-expired": ("expired", False),
        "m-archived": ("archived", False),
        "m-disputed": ("disputed", False),
        "m-pinned-archived": ("archived", True),
    }
    with SessionLocal() as db:
        owner = owner_id(db)
        rows = {}
        for key, (state, pinned) in states.items():
            created = mem_service.change(db, owner, f"Lifecyclegate {key} の記憶", pinned=pinned, source_run="explicit-user-request")
            row = db.get(Memory, created["id"])
            row.lifecycle_state = state
            rows[key] = row
        db.flush()

        found = mem_service.search(db, owner, "Lifecyclegate", debug=True)

        assert {item["id"] for item in found["memories"]} == {rows["m-established"].id, rows["m-pinned-archived"].id}
        assert found["debug"]["considered"] == 2
        assert found["debug"]["lifecycle_excluded"] == 4
        # Review keeps showing what recall hides (§3: hidden from recall, not from review).
        assert {item["id"] for item in mem_service.list_traces(db, owner)} == {row.id for row in rows.values()}
        assert mem_service.retrievable(rows["m-pinned-archived"]) is True
        assert mem_service.retrievable(rows["m-archived"]) is False

        # The same gate holds in the pure ranking contract.
        ranked = mem_service.rank(
            {"lifecyclegate"},
            [({"lifecyclegate"}, now(), None, rows["m-superseded"]), ({"lifecyclegate"}, now(), None, rows["m-established"])],
        )
        assert [item["payload"].id for item in ranked] == [rows["m-established"].id]


def test_render_and_trace_report_the_real_lifecycle(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        explicit = mem_tools.remember(db, owner, content="MIX-agent のデフォルトは英語で話す", explicit_user=True)
        automatic = mem_tools.remember(db, owner, content="MIX-agent は Slack 連携に対応している")
        correction = mem_service.change(db, owner, "コーヒーはミルクを入れて飲む")
        earlier = mem_service.change(db, owner, "コーヒーはブラックで飲む", source_run="explicit-user-request")
        db.flush()
        mem_runtime.supersede(db, owner, old_id=earlier["id"], new_id=correction["id"], reason="好みが変わった")
        db.flush()

        explicit_row = db.get(Memory, explicit["applied"][0]["id"])
        automatic_row = db.get(Memory, automatic["applied"][0]["id"])
        # §3's initial lifecycle: explicit human input is established, the
        # automatic one stays a candidate until something confirms it.
        assert (explicit_row.lifecycle_state, explicit_row.source_kind) == ("established", "user")
        assert (automatic_row.lifecycle_state, automatic_row.source_kind) == ("candidate", "agent")

        view = mem_views.render(explicit_row)
        assert view["lifecycle"] == view["lifecycle_state"] == "established"
        assert (view["role"], view["scope"], view["source_kind"]) == ("fact", "user", "user")

        traced = {item["id"]: item for item in mem_service.list_traces(db, owner)}
        retired = traced[earlier["id"]]
        assert retired["lifecycle_state"] == "superseded"
        assert retired["superseded_by_id"] == correction["id"]
        assert mem_views.render(db.get(Memory, earlier["id"]))["lifecycle_state"] == "superseded"

        debug = mem_runtime.recall(db, owner, {"text": "MIX-agent", "kind": "test"}, debug=True)
        # Honest debug: what the results actually are, not `["active"]`.
        assert debug["debug"]["lifecycles"] == ["candidate", "established"]
        assert debug["debug"]["scopes"] == ["user"]
        assert debug["debug"]["roles"] == ["fact"]
        assert debug["debug"]["lifecycle_excluded"] == 1
