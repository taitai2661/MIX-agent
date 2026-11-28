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


def test_impossible_date_fails_fast_without_scanning_a_year():
    from mix_agent.schedules import next_at

    # "Feb 30" can never match: the old loop tried 527,040 minutes with a fresh
    # ZoneInfo on every iteration before giving up.
    with pytest.raises(ValueError, match="次回実行時刻"):
        next_at("0 0 30 2 *", "Asia/Tokyo", utc(2025, 1, 1))
    with pytest.raises(ValueError, match="次回実行時刻"):
        next_at("0 0 31 4 *", "UTC", utc(2025, 1, 1))


def test_leap_day_still_resolves_within_the_bounded_search():
    from mix_agent.schedules import next_at

    result = next_at("0 0 29 2 *", "UTC", utc(2025, 1, 1))
    assert (result.year, result.month, result.day) == (2028, 2, 29)


def test_dst_spring_forward_skips_the_nonexistent_local_time():
    """02:00 does not exist on 2025-03-09 in New York.

    The date-based fast path used to return 07:00 UTC (03:00 EDT) for that day,
    an instant `tick()` can never match because `matches()` sees local hour 3.
    """
    from mix_agent.schedules import next_at

    expression = "0 2 * * *"
    result = next_at(expression, "America/New_York", utc(2025, 3, 8, 12, 0))
    # The next real 02:00 is two days later (March 10), 06:00 UTC in EDT.
    assert result == utc(2025, 3, 10, 6, 0)
    assert _matches(parse(expression), result, "America/New_York")


def test_dst_fall_back_returns_a_matching_instant():
    """The first 01:30 on the repeated hour is returned and matches the cron."""
    from mix_agent.schedules import next_at

    expression = "30 1 * * *"
    result = next_at(expression, "America/New_York", utc(2025, 11, 1, 12, 0))
    assert result == utc(2025, 11, 2, 5, 30)  # 01:30 EDT, fold=0
    assert _matches(parse(expression), result, "America/New_York")


@pytest.mark.parametrize(
    ("expression", "timezone", "after"),
    [
        ("0 2 * * *", "Europe/Berlin", utc(2025, 3, 29, 12, 0)),
        ("*/15 * * * *", "Asia/Tokyo", utc(2025, 1, 1, 0, 0)),
        ("0 0 29 2 *", "UTC", utc(2025, 1, 1)),
    ],
)
def test_next_always_returns_an_instant_that_matches_the_expression(expression, timezone, after):
    """Invariant: `_next()` and `matches()` must agree, including around DST."""
    from mix_agent.schedules import next_at

    result = next_at(expression, timezone, after)
    assert result >= after
    assert _matches(parse(expression), result, timezone)


def test_matches_uses_cron_weekday_sunday_zero():
    sunday = utc(2025, 1, 5, 0, 0)  # 2025-01-05 is a Sunday.
    assert _matches(parse("0 0 * * 0"), sunday, "UTC")
    assert not _matches(parse("0 0 * * 1"), sunday, "UTC")


def test_matches_is_timezone_aware():
    instant = utc(2025, 1, 5, 21, 30)  # 06:30 JST on Monday Jan 6.
    assert _matches(parse("30 6 * * 1"), instant, "Asia/Tokyo")
    assert not _matches(parse("30 6 * * 1"), instant, "UTC")


def test_reconcile_with_no_enabled_jobs_is_a_noop():
    # `to_launch` used to be scoped inside the per-job loop; with no enabled
    # jobs the function raised UnboundLocalError and killed the scheduler task.
    from mix_agent.schedules import reconcile

    class EmptyDB:
        def scalars(self, _query):
            return iter(())

    launched: list[str] = []
    reconcile(EmptyDB(), launched.append)
    assert launched == []


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
        "61 * * * *",  # invalid minute value
        "* 24 * * *",  # invalid hour value
        "* * 0 * *",  # invalid day-of-month
        "* * * 13 *",  # invalid month
        "* * * * 7",  # invalid weekday
        "* * *",  # wrong field count
        "1/0 * * * *",  # zero step
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
