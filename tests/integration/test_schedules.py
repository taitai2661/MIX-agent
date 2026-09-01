from datetime import datetime, timezone

import pytest

from mix_agent.schedules import matches, next_at, parse


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
