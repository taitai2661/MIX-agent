"""Five-field cron parse/match/jump logic (Phase 3 schedule limits)."""

from datetime import UTC, datetime

import pytest
from mix_agent.schedules import _matches, _next, parse


def utc(y, mo, d, h=0, mi=0):
    return datetime(y, mo, d, h, mi, tzinfo=UTC)


def test_next_jumps_to_the_next_matching_occurrence():
    parsed = parse("0 6 * * 1")  # every Monday 06:00 JST.
    assert _next(parsed, "Asia/Tokyo", utc(2025, 1, 1, 0, 0)) == utc(2025, 1, 5, 21, 0)


def test_next_never_returns_the_boundary_itself():
    parsed = parse("* * * * *")
    assert _next(parsed, "UTC", utc(2025, 1, 1, 0, 0)) == utc(2025, 1, 1, 0, 1)


def test_matches_uses_cron_weekday_sunday_zero():
    sunday = utc(2025, 1, 5, 0, 0)  # 2025-01-05 is a Sunday.
    assert _matches(parse("0 0 * * 0"), sunday, "UTC")
    assert not _matches(parse("0 0 * * 1"), sunday, "UTC")


def test_matches_is_timezone_aware():
    instant = utc(2025, 1, 5, 21, 30)  # 06:30 JST on Monday Jan 6.
    assert _matches(parse("30 6 * * 1"), instant, "Asia/Tokyo")
    assert not _matches(parse("30 6 * * 1"), instant, "UTC")


def test_reconcile_horizon_constants_are_bounded():
    from datetime import timedelta

    from mix_agent.schedules import (
        MAX_MISSED_ROWS,
        MAX_RECONCILE_OCCURRENCES,
        RECONCILE_HORIZON,
    )

    assert RECONCILE_HORIZON <= timedelta(days=30)
    assert 0 < MAX_MISSED_ROWS <= 5000
    assert 0 < MAX_RECONCILE_OCCURRENCES <= 5000


@pytest.mark.parametrize(
    "expression",
    [
        "61 * * * *",      # invalid minute value
        "* 24 * * *",      # invalid hour value
        "* * 0 * *",       # invalid day-of-month
        "* * * 13 *",      # invalid month
        "* * * * 7",       # invalid weekday
        "* * *",           # wrong field count
        "1/0 * * * *",     # zero step
    ],
)
def test_parse_rejects_invalid_expressions(expression):
    with pytest.raises(ValueError):
        parse(expression)


def test_parse_accepts_steps_and_lists():
    parsed = parse("*/5 9-11 1,15 * 1-5")
    assert 0 in parsed[0] and 5 in parsed[0] and 355 not in parsed[0]
    assert 9 in parsed[1] and 11 in parsed[1] and 12 not in parsed[1]
    assert parsed[2] == {1, 15}
    assert parsed[4] == {1, 2, 3, 4, 5}