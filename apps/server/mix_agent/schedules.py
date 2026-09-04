"""Small dependency-free five-field cron scheduler.

The database is the source of truth: process-local polling never decides whether
an occurrence has already been claimed.
"""
from datetime import datetime, timedelta, timezone
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError
from sqlalchemy import select
from sqlalchemy.exc import IntegrityError
from mix_agent.db.models import ScheduledJob, ScheduledRun, Notification, Run, now

FIELDS = ((0, 59), (0, 23), (1, 31), (1, 12), (0, 6))

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

def matches(expression, instant, tz):
    minute, hour, day, month, weekday = parse(expression)
    local = instant.astimezone(ZoneInfo(tz))
    # cron weekday uses Sunday=0; Python Monday=0.
    return (local.minute in minute and local.hour in hour and local.day in day and local.month in month and (local.weekday() + 1) % 7 in weekday)

def next_at(expression, tz, after=None):
    parse(expression)
    try: ZoneInfo(tz)
    except ZoneInfoNotFoundError as exc: raise ValueError("タイムゾーンが不正です") from exc
    candidate = (after or now()).astimezone(timezone.utc).replace(second=0, microsecond=0) + timedelta(minutes=1)
    for _ in range(527_040):
        if matches(expression, candidate, tz): return candidate
        candidate += timedelta(minutes=1)
    raise ValueError("次回実行時刻を計算できません")

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
        except Exception:
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
        except Exception as exc:
            db.rollback(); scheduled = db.get(ScheduledRun, scheduled.id)
            if scheduled:
                scheduled.status = "failed"; scheduled.data = {"reason": str(exc)[:300]}; notify(db, job.owner_id, "schedule.failed", data["name"], scheduled); db.commit()

def reconcile(db, launch):
    """Mark downtime occurrences missed; optional catch-up runs only the latest."""
    current = now().replace(second=0, microsecond=0)
    for job in db.scalars(select(ScheduledJob).where(ScheduledJob.data["enabled"].as_boolean() == True)):
        latest = db.scalar(select(ScheduledRun).where(ScheduledRun.job_id == job.id).order_by(ScheduledRun.scheduled_at.desc()))
        cursor = (latest.scheduled_at if latest else job.created_at).replace(second=0, microsecond=0)
        occurrences = []
        while cursor < current:
            cursor += timedelta(minutes=1)
            if matches(job.data["cron"], cursor, job.data["timezone"]): occurrences.append(cursor)
        if not occurrences: continue
        chosen = occurrences[-1] if job.data.get("catch_up") else None
        for occurrence in occurrences:
            scheduled = claim(db, job, occurrence)
            if not scheduled: continue
            if occurrence != chosen:
                scheduled.status = "missed"; scheduled.data = {"reason": "サーバー停止中"}; notify(db, job.owner_id, "schedule.missed", job.data["name"], scheduled)
            else:
                from mix_agent.api.routes import enqueue_scheduled_run
                run = enqueue_scheduled_run(db, job.owner_id, job, scheduled); launch(run.id)
        db.commit()
