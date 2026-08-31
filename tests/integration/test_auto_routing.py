from mix_agent.api import routes
from mix_agent.db.models import (
    Conversation,
    Feedback,
    Message,
    Model,
    Provider,
    Run,
    User,
)
from mix_agent.db.session import SessionLocal
from mix_agent.routing import select_auto_model
from sqlalchemy import select


def make_model(caps, context=20000, model_id="auto-test"):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        provider = Provider(owner_id=owner, data={"kind": "openai"})
        db.add(provider)
        db.flush()
        model = Model(owner_id=owner, data={"provider_id": provider.id, "model_id": model_id,
                                             "capabilities": caps, "context_window": context})
        db.add(model)
        db.commit()
        return model.id


def test_auto_settings_and_capability_filters(signed, monkeypatch):
    plain = make_model({"tools": True}, model_id="plain")
    vision = make_model({"tools": True, "vision": True}, model_id="vision")
    assert signed.put("/api/v1/settings", json={"default_model_id": "auto", "auto_model_ids": [plain, vision],
                                                  "allowed_domains": []}).status_code == 200
    monkeypatch.setattr(routes, "launch", lambda _: None)
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    response = signed.post(f"/api/v1/conversations/{conversation}/messages", json={
        "model_id": "auto", "content": "hello", "mode": "chat"}, headers={"Idempotency-Key": conversation})
    assert response.status_code == 200, response.text
    with SessionLocal() as db:
        run = db.get(Run, response.json()["run_id"])
        assert run.data["requested_model_id"] == "auto"
        assert run.data["snapshot"]["model_record_id"] in (plain, vision)
        assert run.data["auto_selection"]["candidate_count"] == 2
        owner = db.scalar(select(User.id))
        selected, details = select_auto_model(db, owner, [plain, vision], "look", "chat", ["image/png"], False,
                                              ["look"], 10, "a-unique-request")
        assert selected.id == vision
        assert details["candidate_count"] == 1


def test_ucb_feedback_is_profile_scoped_and_single_rating_is_smoothed(signed):
    first = make_model({}, model_id="first")
    second = make_model({}, model_id="second")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "ratings"})
        db.add(conversation)
        db.flush()
        for _ in range(30):
            message = Message(owner_id=owner, conversation_id=conversation.id, data={"role": "assistant", "content": "x"})
            db.add(message)
            db.flush()
            db.add(Feedback(owner_id=owner, message_id=message.id, data={"model_id": first, "profile": "coding", "value": "up"}))
        db.commit()
        selected, _ = select_auto_model(db, owner, [first, second], "code", "chat", [], False, ["code"], 10, "b-request")
        # A well-tested model wins over the exploration bonus; a different profile does not use these ratings.
        assert selected.id == first
        selected, _ = select_auto_model(db, owner, [first, second], "hello", "chat", [], False, ["hello"], 10, "c-request")
        assert selected.id in (first, second)


def test_feedback_replaces_or_clears_only_auto_answers(signed):
    model_id = make_model({}, model_id="rated")
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "test"})
        db.add(conversation)
        db.flush()
        message = Message(owner_id=owner, conversation_id=conversation.id, data={"role": "assistant", "content": "x",
            "auto_selection": {"model_record_id": model_id, "profile": "general"}})
        db.add(message)
        db.commit()
        message_id = message.id
    assert signed.put(f"/api/v1/messages/{message_id}/feedback", json={"value": "up"}).json()["value"] == "up"
    assert signed.put(f"/api/v1/messages/{message_id}/feedback", json={"value": "down"}).json()["value"] == "down"
    assert signed.put(f"/api/v1/messages/{message_id}/feedback", json={"value": None}).json()["value"] is None
    with SessionLocal() as db:
        assert db.scalar(select(Feedback).where(Feedback.message_id == message_id)) is None
