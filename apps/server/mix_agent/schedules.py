"""Small dependency-free five-field cron scheduler.

The database is the source of truth: process-local polling never decides whether
an occurrence has already been claimed.
"""
from datetime import UTC, datetime, timedelta
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from sqlalchemy import select
from sqlalchemy.exc import IntegrityError

from mix_agent.db.models import Notification, ScheduledJob, ScheduledRun, now

FIELDS = ((0, 59), (0, 23), (1, 31), (1, 12), (0, 6))

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
        if step < 1: raise ValueError("step")
        if base == "*": start, end = lower, upper
        elif "-" in base:
            start, end = map(int, base.split("-", 1))
        else: start = end = int(base)
        if start < lower or end > upper or start > end: raise ValueError("range")
        result.update(range(start, end + 1, step))
    return result

def parse(expression):
    parts = expression.split()
    if len(parts) != 5: raise ValueError("Cron式は5フィールドで入力してください")
    try: return [_part(part, *bounds) for part, bounds in zip(parts, FIELDS)]
    except (ValueError, TypeError): raise ValueError("Cron式が不正です")

def _matches(parsed, instant, tz):
    minute, hour, day, month, weekday = parsed
    local = instant.astimezone(ZoneInfo(tz))
    # cron weekday uses Sunday=0; Python Monday=0.
    return (local.minute in minute and local.hour in hour and local.day in day and local.month in month and (local.weekday() + 1) % 7 in weekday)

def matches(expression, instant, tz):
    return _matches(parse(expression), instant, tz)

def _next(parsed, tz, after=None):
    candidate = (after or now()).astimezone(UTC).replace(second=0, microsecond=0) + timedelta(minutes=1)
    for _ in range(527_040):
        if _matches(parsed, candidate, tz): return candidate
        candidate += timedelta(minutes=1)
    raise ValueError("次回実行時刻を計算できません")

def next_at(expression, tz, after=None):
    parsed = parse(expression)
    try: ZoneInfo(tz)
    except ZoneInfoNotFoundError as exc: raise ValueError("タイムゾーンが不正です") from exc
    return _next(parsed, tz, after)

def notify(db, owner, kind, title, scheduled_run=None):
    db.add(Notification(owner_id=owner, data={"kind": kind, "title": title, "scheduled_run_id": scheduled_run.id if scheduled_run else None}))

def claim(db, job, scheduled_at):
    row = ScheduledRun(owner_id=job.owner_id, job_id=job.id, scheduled_at=scheduled_at, data={"attempt": 0})
    db.add(row)
    try: db.flush(); return row
    except IntegrityError: db.rollback(); return None

def tick(db, launch):
    current = now().replace(second=0, microsecond=0)
    # Retries reuse the same occurrence record, preserving its idempotency lock.
    for scheduled in db.scalars(select(ScheduledRun).where(ScheduledRun.status == "retrying")):
        retry_at = scheduled.data.get("retry_at")
        if not retry_at or datetime.fromisoformat(retry_at) > now(): continue
        job = db.get(ScheduledJob, scheduled.job_id)
        if not job or not job.data.get("enabled"): continue
        from mix_agent.api.routes import enqueue_scheduled_run
        try:
            run = enqueue_scheduled_run(db, job.owner_id, job, scheduled); db.commit(); launch(run.id)
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            db.rollback()
    for job in db.scalars(select(ScheduledJob).where(ScheduledJob.data["enabled"].as_boolean() == True)):
        data = job.data
        try: due = matches(data["cron"], current, data["timezone"])
        except (KeyError, ValueError): continue
        if not due: continue
        scheduled = claim(db, job, current)
        if not scheduled: continue
        active = db.scalar(select(ScheduledRun).where(ScheduledRun.job_id == job.id, ScheduledRun.status.in_(["pending", "running", "retrying"]), ScheduledRun.id != scheduled.id))
        if active:
            scheduled.status = "skipped"; scheduled.data = {"reason": "前回の実行中"}; notify(db, job.owner_id, "schedule.skipped", data["name"], scheduled); db.commit(); continue
        from mix_agent.api.routes import enqueue_scheduled_run
        try:
            run = enqueue_scheduled_run(db, job.owner_id, job, scheduled); db.commit(); launch(run.id)
        except Exception as exc:  # noqa: BLE001 - intentionally classified; never leak raw details
            db.rollback(); scheduled = db.get(ScheduledRun, scheduled.id)
            if scheduled:
                scheduled.status = "failed"; scheduled.data = {"reason": str(exc)[:300]}; notify(db, job.owner_id, "schedule.failed", data["name"], scheduled); db.commit()

def reconcile(db, launch):
    """Mark downtime occurrences missed; optional catch-up runs only the latest.

    Occurrences are found by jumping from match to match (never scanning every
    minute), the look-back window is bounded, and the number of historical rows
    created per job is capped so a long-dormant job cannot hang startup.
    """
    current = now().replace(second=0, microsecond=0)
    for job in db.scalars(select(ScheduledJob).where(ScheduledJob.data["enabled"].as_boolean() == True)):
        try:
            parsed, tz = parse(job.data["cron"]), job.data["timezone"]
            ZoneInfo(tz)
        except (KeyError, ValueError, ZoneInfoNotFoundError):
            continue
        latest = db.scalar(select(ScheduledRun).where(ScheduledRun.job_id == job.id).order_by(ScheduledRun.scheduled_at.desc()))
        cursor = (latest.scheduled_at if latest else job.created_at).replace(second=0, microsecond=0)
        cursor = max(cursor, current - RECONCILE_HORIZON)
        occurrences = []
        while len(occurrences) < MAX_RECONCILE_OCCURRENCES:
            try: candidate = _next(parsed, tz, cursor)
            except ValueError: break
            if candidate >= current: break
            occurrences.append(candidate); cursor = candidate
        if not occurrences: continue
        chosen = occurrences[-1] if job.data.get("catch_up") else None
        from mix_agent.api.routes import enqueue_scheduled_run
        for occurrence in occurrences[-MAX_MISSED_ROWS:]:
            scheduled = claim(db, job, occurrence)
            if not scheduled: continue
            if occurrence != chosen:
                scheduled.status = "missed"; scheduled.data = {"reason": "サーバー停止中"}; notify(db, job.owner_id, "schedule.missed", job.data["name"], scheduled)
            else:
                run = enqueue_scheduled_run(db, job.owner_id, job, scheduled); launch(run.id)
        db.commit()
