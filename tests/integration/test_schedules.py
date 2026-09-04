from datetime import datetime, timezone

import pytest

from mix_agent.schedules import matches, next_at, parse
from mix_agent.tools.execute import execute
from mix_agent.tools.registry import BUILTINS
from sqlalchemy import select
from mix_agent.db.models import Conversation, Run, ScheduledJob, User, uid
from mix_agent.db.session import SessionLocal


def test_five_field_cron_validation_and_next_time():
    assert parse("*/15 9-17 * * 1-5")
    after = datetime(2026, 9, 1, 0, 7, tzinfo=timezone.utc)
    result = next_at("*/15 9 * * *", "Asia/Tokyo", after)
    assert result.isoformat() == "2026-09-01T00:15:00+00:00"
    assert matches("*/15 9 * * *", result, "Asia/Tokyo")


def test_cron_rejects_invalid_expression_and_timezone():
    with pytest.raises(ValueError): parse("* * *")
    with pytest.raises(ValueError): next_at("0 0 * * *", "Not/AZone")


def test_scheduled_job_api_validates_and_persists_per_owner(signed):
    conversation = signed.post("/api/v1/conversations", json={}).json()["id"]
    payload = {"name": "朝の確認", "target_type": "conversation", "target_id": conversation,
               "prompt": "状況を確認して", "cron": "0 9 * * *", "timezone": "Asia/Tokyo", "enabled": True, "catch_up": False}
    created = signed.post("/api/v1/scheduled-jobs", json=payload)
    assert created.status_code == 200, created.text
    listing = signed.get("/api/v1/scheduled-jobs")
    assert listing.status_code == 200
    assert listing.json()[0]["data"]["name"] == "朝の確認"
    assert signed.post("/api/v1/scheduled-jobs", json={**payload, "cron": "bad"}).status_code == 422


@pytest.mark.asyncio
async def test_agent_schedule_tools_manage_only_owned_jobs(signed):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "Test"})
        db.add(conversation)
        db.flush()
        run = Run(owner_id=owner, conversation_id=conversation.id, request_key=uid(), data={"snapshot": {}})
        db.add(run); db.flush()
        create = next(tool for tool in BUILTINS if tool["id"] == "schedule_create")
        created = await execute(db, run, create, {"name": "朝", "target_type": "conversation", "target_id": conversation.id, "prompt": "確認して", "cron": "0 9 * * *"})
        assert created["created"] is True
        job = db.get(ScheduledJob, created["id"])
        assert job and job.data["timezone"] == "Asia/Tokyo"
        update = next(tool for tool in BUILTINS if tool["id"] == "schedule_update")
        updated = await execute(db, run, update, {"id": job.id, "enabled": False})
        assert updated["enabled"] is False
        listing = await execute(db, run, next(tool for tool in BUILTINS if tool["id"] == "schedule_list"), {})
        assert listing["jobs"][0]["id"] == job.id
        deleted = await execute(db, run, next(tool for tool in BUILTINS if tool["id"] == "schedule_delete"), {"id": job.id})
        assert deleted == {"id": job.id, "deleted": True}
