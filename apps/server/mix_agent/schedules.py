"""Small dependency-free five-field cron scheduler.

The database is the source of truth: process-local polling never decides whether
an occurrence has already been claimed.
"""

import calendar
from datetime import UTC, datetime, timedelta
from functools import lru_cache
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from sqlalchemy import select
from sqlalchemy.exc import IntegrityError

from mix_agent.db.models import Notification, ScheduledJob, ScheduledRun, now

try:  # Optional fast jump for valid expressions; the bounded search is the fallback.
    from croniter import croniter as _croniter
except ImportError:  # pragma: no cover - not a declared dependency in this image
    _croniter = None

FIELDS = ((0, 59), (0, 23), (1, 31), (1, 12), (0, 6))
# Leap day needs at most 8 years to recur; an unsatisfiable expression is
# rejected up front by _satisfiable instead of scanning minute by minute.
NEXT_SEARCH_YEARS = 8


@lru_cache(maxsize=64)
def _zone(tz):
    """ZoneInfo is expensive to construct; the old loop rebuilt it per minute."""
    return ZoneInfo(tz)


# Startup reconciliation bounds: never look further back than this window and
# never create more than this many historical occurrence rows per job.
RECONCILE_HORIZON = timedelta(days=30)
MAX_RECONCILE_OCCURRENCES = 5000
MAX_MISSED_ROWS = 200


def _part(value, lower, upper):
    result = set()
    for token in value.split(","):
        base, _, step = token.partition("/")
        step = int(step) if step else 1
        if step < 1:
            raise ValueError("step")
        if base == "*":
            start, end = lower, upper
        elif "-" in base:
            start, end = map(int, base.split("-", 1))
        else:
            start = end = int(base)
        if start < lower or end > upper or start > end:
            raise ValueError("range")
        result.update(range(start, end + 1, step))
    return result


def parse(expression):
    parts = expression.split()
    if len(parts) != 5:
        raise ValueError("Cron式は5フィールドで入力してください")
    try:
        return [_part(part, *bounds) for part, bounds in zip(parts, FIELDS, strict=False)]
    except (ValueError, TypeError):
        raise ValueError("Cron式が不正です")


def _matches(parsed, instant, tz):
    minute, hour, day, month, weekday = parsed
    local = instant.astimezone(_zone(tz))
    # cron weekday uses Sunday=0; Python Monday=0.
    return (
        local.minute in minute
        and local.hour in hour
        and local.day in day
        and local.month in month
        and (local.weekday() + 1) % 7 in weekday
    )


def matches(expression, instant, tz):
    return _matches(parse(expression), instant, tz)


def _satisfiable(parsed, start_year):
    """Whether any selected (month, day) pair can exist in the near future."""
    minute, hour, day, month, weekday = parsed
    if not (minute and hour and day and month and weekday):
        return False
    for year in range(start_year, start_year + NEXT_SEARCH_YEARS):
        if any(
            1 <= selected_day <= calendar.monthrange(year, selected_month)[1]
            for selected_month in month
            for selected_day in day
        ):
            return True
    return False


def _expression(parsed):
    """Canonical five-field expression used for the optional croniter jump."""
    return " ".join(",".join(str(value) for value in sorted(field)) for field in parsed)


def _next(parsed, tz, after=None):
    candidate = (after or now()).astimezone(UTC).replace(second=0, microsecond=0) + timedelta(minutes=1)
    if not _satisfiable(parsed, candidate.year):
        raise ValueError("次回実行時刻を計算できません")
    if _croniter is not None:
        try:
            guess = _croniter(_expression(parsed), candidate.replace(tzinfo=None)).get_next(datetime)
            guess = guess.replace(tzinfo=_zone(tz)).astimezone(UTC)
            if guess >= candidate and _matches(parsed, guess, tz):
                return guess
        except Exception:  # noqa: BLE001, S110 - optional accelerator; bounded search follows
            pass
    minute, hour, day, month, weekday = parsed
    zone = _zone(tz)
    current = candidate.astimezone(zone).date()
    deadline = current + timedelta(days=NEXT_SEARCH_YEARS * 366)
    while current <= deadline:
        if current.month in month and current.day in day and (current.weekday() + 1) % 7 in weekday:
            for selected_hour in sorted(hour):
                for selected_minute in sorted(minute):
                    # ZoneInfo maps a *nonexistent* local time (spring-forward
                    # gap, e.g. 02:30 on a DST start day) to a real UTC instant
                    # whose local wall clock is a different hour. tick()/reconcile()
                    # decide with `_matches`, so returning that instant here would
                    # advertise a next run that can never fire (and reconcile
                    # would create a bogus missed occurrence). Verify before
                    # returning and let the loop advance to the next real match.
                    try:
                        value = datetime(
                            current.year,
                            current.month,
                            current.day,
                            selected_hour,
                            selected_minute,
                            tzinfo=zone,
                        ).astimezone(UTC)
                    except (ValueError, OverflowError):  # pragma: no cover - defensive
                        continue
                    if value >= candidate and _matches(parsed, value, tz):
                        return value
        current += timedelta(days=1)
    raise ValueError("次回実行時刻を計算できません")


def next_at(expression, tz, after=None):
    parsed = parse(expression)
    try:
        ZoneInfo(tz)
    except ZoneInfoNotFoundError as exc:
        raise ValueError("タイムゾーンが不正です") from exc
    return _next(parsed, tz, after)


def notify(db, owner, kind, title, scheduled_run=None):
    db.add(
        Notification(
            owner_id=owner,
            data={
                "kind": kind,
                "title": title,
                "scheduled_run_id": scheduled_run.id if scheduled_run else None,
            },
        )
    )


def claim(db, job, scheduled_at):
    row = ScheduledRun(owner_id=job.owner_id, job_id=job.id, scheduled_at=scheduled_at, data={"attempt": 0})
    try:
        # Use a SAVEPOINT so a duplicate occurrence only rolls back this
        # insert; a full db.rollback() would discard the caller's pending work.
        with db.begin_nested():
            db.add(row)
            db.flush()
    except IntegrityError:
        return None
    return row


def tick(db, launch):
    current = now().replace(second=0, microsecond=0)
    # Retries reuse the same occurrence record, preserving its idempotency lock.
    for scheduled in db.scalars(select(ScheduledRun).where(ScheduledRun.status == "retrying")):
        raw_retry = scheduled.data.get("retry_at")
        try:
            retry_at = datetime.fromisoformat(raw_retry) if raw_retry else None
        except (ValueError, TypeError):
            continue
        if retry_at is None:
            continue
        if retry_at.tzinfo is None:
            retry_at = retry_at.replace(tzinfo=UTC)
        if retry_at > now():
            continue
        job = db.get(ScheduledJob, scheduled.job_id)
        if not job or not job.data.get("enabled"):
            continue
        from mix_agent.api.helpers.scheduling import enqueue_scheduled_run

        try:
            run = enqueue_scheduled_run(db, job.owner_id, job, scheduled)
            db.commit()
            launch(run.id)
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            db.rollback()
    for job in db.scalars(select(ScheduledJob).where(ScheduledJob.data["enabled"].as_boolean().is_(True))):
        data = job.data
        try:
            due = matches(data["cron"], current, data["timezone"])
        except (KeyError, ValueError):
            continue
        if not due:
            continue
        scheduled = claim(db, job, current)
        if not scheduled:
            continue
        active = db.scalar(
            select(ScheduledRun).where(
                ScheduledRun.job_id == job.id,
                ScheduledRun.status.in_(["pending", "running", "retrying"]),
                ScheduledRun.id != scheduled.id,
            )
        )
        if active:
            scheduled.status = "skipped"
            scheduled.data = {"reason": "前回の実行中"}
            notify(db, job.owner_id, "schedule.skipped", data["name"], scheduled)
            db.commit()
            continue
        from mix_agent.api.helpers.scheduling import enqueue_scheduled_run

        try:
            run = enqueue_scheduled_run(db, job.owner_id, job, scheduled)
            db.commit()
            launch(run.id)
        except Exception as exc:  # noqa: BLE001 - intentionally classified; never leak raw details
            db.rollback()
            scheduled = db.get(ScheduledRun, scheduled.id)
            if scheduled:
                scheduled.status = "failed"
                scheduled.data = {"reason": str(exc)[:300]}
                notify(db, job.owner_id, "schedule.failed", data["name"], scheduled)
                db.commit()


def reconcile(db, launch):
    """Mark downtime occurrences missed; optional catch-up runs only the latest.

    Occurrences are found by jumping from match to match (never scanning every
    minute), the look-back window is bounded, and the number of historical rows
    created per job is capped so a long-dormant job cannot hang startup.
    """
    current = now().replace(second=0, microsecond=0)
    to_launch: list[str] = []
    for job in db.scalars(select(ScheduledJob).where(ScheduledJob.data["enabled"].as_boolean().is_(True))):
        try:
            parsed, tz = parse(job.data["cron"]), job.data["timezone"]
            ZoneInfo(tz)
        except (KeyError, ValueError, ZoneInfoNotFoundError):
            continue
        latest = db.scalar(
            select(ScheduledRun)
            .where(ScheduledRun.job_id == job.id)
            .order_by(ScheduledRun.scheduled_at.desc())
        )
        cursor = (latest.scheduled_at if latest else job.created_at).replace(second=0, microsecond=0)
        cursor = max(cursor, current - RECONCILE_HORIZON)
        occurrences = []
        while len(occurrences) < MAX_RECONCILE_OCCURRENCES:
            try:
                candidate = _next(parsed, tz, cursor)
            except ValueError:
                break
            if candidate >= current:
                break
            occurrences.append(candidate)
            cursor = candidate
        if not occurrences:
            continue
        chosen = occurrences[-1] if job.data.get("catch_up") else None
        from mix_agent.api.helpers.scheduling import enqueue_scheduled_run

        for occurrence in occurrences[-MAX_MISSED_ROWS:]:
            scheduled = claim(db, job, occurrence)
            if not scheduled:
                continue
            if occurrence != chosen:
                scheduled.status = "missed"
                scheduled.data = {"reason": "サーバー停止中"}
                notify(db, job.owner_id, "schedule.missed", job.data["name"], scheduled)
            else:
                run = enqueue_scheduled_run(db, job.owner_id, job, scheduled)
                # Defer launch until after commit (same order as tick()): the
                # run row must be durable before the driver reads it.
                to_launch.append(run.id)
        db.commit()
    for run_id in to_launch:
        launch(run_id)
