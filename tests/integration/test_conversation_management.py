from datetime import timedelta

from mix_agent.api.routes import purge_expired_conversations
from mix_agent.db.models import Conversation, Message, now
from mix_agent.db.session import SessionLocal


def test_folder_state_search_export_and_trash(signed):
    folder = signed.post("/api/v1/conversation-folders", json={"name": "仕事"})
    assert folder.status_code == 200
    conversation = signed.post("/api/v1/conversations", json={"title": "設計メモ"}).json()
    key = conversation["id"]
    state = signed.patch(f"/api/v1/conversations/{key}/state", json={"folder_id": folder.json()["id"], "pinned": True})
    assert state.json()["data"]["pinned"] is True
    with SessionLocal() as db:
        db.add(Message(owner_id=conversation_owner(db, key), conversation_id=key, data={"role": "user", "content": "検索できる本文"}))
        db.commit()
    assert [row["id"] for row in signed.get("/api/v1/conversations?q=本文").json()] == [key]
    markdown = signed.get(f"/api/v1/conversations/{key}/markdown")
    assert markdown.status_code == 200 and "検索できる本文" in markdown.text
    assert signed.delete(f"/api/v1/conversations/{key}").status_code == 200
    assert [row["id"] for row in signed.get("/api/v1/conversations?state=trash").json()] == [key]
    assert signed.post(f"/api/v1/conversations/{key}/restore").status_code == 200
    assert signed.delete(f"/api/v1/conversation-folders/{folder.json()['id']}").status_code == 200
    assert signed.get("/api/v1/conversations").json()[0]["data"].get("folder_id") is None


def test_conversations_are_sorted_by_recent_activity_with_pinned_first(signed):
    old = signed.post("/api/v1/conversations", json={"title": "古い会話"}).json()
    recent = signed.post("/api/v1/conversations", json={"title": "新しい会話"}).json()
    pinned = signed.post("/api/v1/conversations", json={"title": "ピン留め会話"}).json()
    with SessionLocal() as db:
        for key, timestamp in ((old["id"], "2026-09-01T01:00:00+00:00"), (recent["id"], "2026-09-01T03:00:00+00:00"), (pinned["id"], "2026-09-01T02:00:00+00:00")):
            row = db.get(Conversation, key)
            row.data = {**row.data, "last_message_at": timestamp}
        db.get(Conversation, pinned["id"]).data = {**db.get(Conversation, pinned["id"]).data, "pinned": True}
        db.commit()

    rows = signed.get("/api/v1/conversations").json()
    expected = {old["id"], recent["id"], pinned["id"]}
    assert [row["id"] for row in rows if row["id"] in expected] == [pinned["id"], recent["id"], old["id"]]


def test_expired_trash_is_purged(signed):
    conversation = signed.post("/api/v1/conversations", json={}).json()
    key = conversation["id"]
    assert signed.delete(f"/api/v1/conversations/{key}").status_code == 200
    with SessionLocal() as db:
        row = db.get(Conversation, key)
        row.data = {**row.data, "deleted_at": (now() - timedelta(days=31)).isoformat()}
        db.commit()
        purge_expired_conversations(db)
        assert db.get(Conversation, key) is None


def conversation_owner(db, key):
    return db.get(Conversation, key).owner_id
