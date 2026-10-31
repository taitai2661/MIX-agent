from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select

from mix_agent.api.common import (
    audit,
    context,
    own,
    public,
)
from mix_agent.api.schemas import (
    ScheduledJobInput,
)
from mix_agent.db.models import (
    Agent,
    Conversation,
    Message,
    ScheduledJob,
    ScheduledRun,
    now,
)
from mix_agent.runs.engine import launch

from mix_agent.api.helpers.scheduling import enqueue_scheduled_run

router = APIRouter(prefix="/api/v1")


@router.get("/scheduled-jobs")
def list_scheduled_jobs(ctx=Depends(context)):
    db, s = ctx
    from mix_agent.schedules import next_at

    rows = []
    for job in db.scalars(
        select(ScheduledJob)
        .where(ScheduledJob.owner_id == s.owner_id)
        .order_by(ScheduledJob.created_at.desc())
    ):
        value = public(job)
        try:
            value["data"]["next_at"] = (
                next_at(job.data["cron"], job.data["timezone"]).isoformat()
                if job.data.get("enabled")
                else None
            )
        except ValueError:
            value["data"]["next_at"] = None
        rows.append(value)
    return rows


@router.post("/scheduled-jobs")
def create_scheduled_job(body: ScheduledJobInput, ctx=Depends(context)):
    db, s = ctx
    from mix_agent.schedules import next_at

    try:
        next_at(body.cron, body.timezone)
    except ValueError as exc:
        raise HTTPException(422, str(exc))
    if body.target_type == "agent":
        own(db, Agent, body.target_id, s.owner_id)
    else:
        own(db, Conversation, body.target_id, s.owner_id)
    row = ScheduledJob(owner_id=s.owner_id, data=body.model_dump())
    db.add(row)
    db.flush()
    audit(db, s.owner_id, "scheduled-job.create", row.id)
    db.commit()
    return public(row)


@router.patch("/scheduled-jobs/{key}")
def edit_scheduled_job(key: str, body: ScheduledJobInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, ScheduledJob, key, s.owner_id)
    from mix_agent.schedules import next_at

    try:
        next_at(body.cron, body.timezone)
    except ValueError as exc:
        raise HTTPException(422, str(exc))
    if body.target_type == "agent":
        own(db, Agent, body.target_id, s.owner_id)
    else:
        own(db, Conversation, body.target_id, s.owner_id)
    row.data = body.model_dump()
    audit(db, s.owner_id, "scheduled-job.update", key)
    db.commit()
    return public(row)


@router.delete("/scheduled-jobs/{key}")
def delete_scheduled_job(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, ScheduledJob, key, s.owner_id)
    db.delete(row)
    audit(db, s.owner_id, "scheduled-job.delete", key)
    db.commit()
    return {"ok": True}


@router.post("/scheduled-jobs/{key}/run")
def run_scheduled_job(key: str, ctx=Depends(context)):
    db, s = ctx
    job = own(db, ScheduledJob, key, s.owner_id)
    from mix_agent.schedules import claim

    scheduled = claim(db, job, now())
    if not scheduled:
        raise HTTPException(409, "この実行はすでに開始されています")
    run = enqueue_scheduled_run(db, s.owner_id, job, scheduled)
    audit(db, s.owner_id, "scheduled-job.run", key)
    db.commit()
    launch(run.id)
    return {"scheduled_run_id": scheduled.id, "run_id": run.id}


@router.get("/scheduled-jobs/{key}/runs")
def list_scheduled_runs(key: str, ctx=Depends(context)):
    db, s = ctx
    own(db, ScheduledJob, key, s.owner_id)
    rows = []
    for row in db.scalars(
        select(ScheduledRun)
        .where(ScheduledRun.job_id == key)
        .order_by(ScheduledRun.scheduled_at.desc())
        .limit(100)
    ):
        value = public(row)
        value["scheduled_at"] = row.scheduled_at.isoformat()
        value["status"] = row.status
        if row.run_id:
            message = db.scalar(select(Message).where(Message.data["run_id"].as_string() == row.run_id))
            value["answer"] = message.data.get("content") if message else None
        rows.append(value)
    return rows
