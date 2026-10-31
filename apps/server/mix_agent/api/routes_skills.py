import asyncio

from fastapi import APIRouter, Depends, File, HTTPException, Response, UploadFile
from fastapi.responses import PlainTextResponse
from sqlalchemy import select

from mix_agent import config
from mix_agent.api.common import (
    context,
    own,
    public,
    skill_filename as _skill_filename,
)
from mix_agent.api.schemas import (
    SkillImportInput,
    SkillInput,
)
from mix_agent.db.models import (
    Skill,
    SkillRevision,
)
from mix_agent.skills import package as skill_package
from mix_agent.skills import service as skills

router = APIRouter(prefix="/api/v1")

# Backward-compatible re-exports for external callers (engine.py, scheduler, etc.)


@router.get("/skills")
def list_skills(q: str = "", ctx=Depends(context)):
    db, s = ctx
    return [
        public(r)
        for r in db.scalars(select(Skill).where(Skill.owner_id == s.owner_id))
        if q.casefold()
        in (
            r.data.get("name", "") + " " + r.data.get("description", "") + " " + r.data.get("content", "")
        ).casefold()
    ]


@router.post("/skills")
def add_skill(body: SkillInput, ctx=Depends(context)):
    db, s = ctx
    try:
        result = skills.change(db, s.owner_id, **body.model_dump())
    except ValueError as exc:
        # A name or description the SKILL.md rule rejects; `detail` as a plain
        # string is what the WebView renders verbatim.
        raise HTTPException(422, str(exc)) from exc
    db.commit()
    return {
        "id": result["id"],
        "data": {k: v for k, v in result.items() if k != "id"},
        "created_at": db.get(Skill, result["id"]).created_at.isoformat(),
    }


@router.patch("/skills/{key}")
def edit_skill(key: str, body: SkillInput, ctx=Depends(context)):
    db, s = ctx
    own(db, Skill, key, s.owner_id)
    try:
        result = skills.change(db, s.owner_id, skill_id=key, **body.model_dump())
    except ValueError as exc:
        raise HTTPException(422, str(exc)) from exc
    db.commit()
    return public(db.get(Skill, result["id"]))


@router.get("/skills/{key}/revisions")
def skill_history(key: str, ctx=Depends(context)):
    db, s = ctx
    own(db, Skill, key, s.owner_id)
    rows = db.scalars(
        select(SkillRevision)
        .where(SkillRevision.owner_id == s.owner_id, SkillRevision.data["skill_id"].as_string() == key)
        .order_by(SkillRevision.created_at.desc())
    )
    return [public(r) for r in rows]


@router.post("/skills/{key}/restore/{revision}")
def restore_skill(key: str, revision: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Skill, key, s.owner_id)
    rev = own(db, SkillRevision, revision, s.owner_id)
    if rev.data["skill_id"] != key:
        raise HTTPException(404)
    db.add(SkillRevision(owner_id=s.owner_id, data={"skill_id": key, "previous": row.data}))
    row.data = rev.data["previous"]
    db.commit()
    return public(row)


@router.post("/skills/import")
def import_skill(body: SkillImportInput, ctx=Depends(context)):
    db, s = ctx
    try:
        parsed = skill_package.parse_skill_md(body.text)
    except skill_package.SkillFormatError as exc:
        raise HTTPException(422, str(exc)) from exc
    result = skills.import_package(db, s.owner_id, parsed, source="upload")
    db.commit()
    return public(db.get(Skill, result["id"]))


@router.post("/skills/import/archive")
async def import_skill_archive(file: UploadFile = File(...), ctx=Depends(context)):
    db, s = ctx
    raw = await file.read(skill_package.ARCHIVE_MAX + 1)
    if len(raw) > skill_package.ARCHIVE_MAX:
        raise HTTPException(413, "アーカイブは5MBまでです")
    try:
        parsed = await asyncio.to_thread(skill_package.parse_archive, raw)
    except skill_package.SkillFormatError as exc:
        raise HTTPException(422, str(exc)) from exc
    filename = (file.filename or "")[:200]
    results = [
        skills.import_package(db, s.owner_id, item, source="archive", source_path=filename) for item in parsed
    ]
    db.commit()
    return [public(db.get(Skill, item["id"])) for item in results]


@router.post("/skills/import/directory")
async def import_skill_directory(ctx=Depends(context)):
    db, s = ctx
    packages, failures = await asyncio.to_thread(skill_package.scan_directory_report, config.SKILLS_DIR)
    if failures:
        # Importing while some entries failed would report success for a
        # directory the user expects to be fully taken; show the reasons.
        detail = "; ".join(f"{item['path']}: {item['reason']}" for item in failures[:5])
        raise HTTPException(422, detail)
    results = [
        skills.import_package(
            db, s.owner_id, item, source="directory", source_path=str(config.SKILLS_DIR / item["name"])
        )
        for item in packages
    ]
    db.commit()
    return {"imported": len(results), "skills": [public(db.get(Skill, item["id"])) for item in results]}


@router.get("/skills/{key}/export")
def export_skill(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Skill, key, s.owner_id)
    return PlainTextResponse(
        skills.export_markdown(row.data),
        media_type="text/markdown; charset=utf-8",
        headers={"Content-Disposition": f'attachment; filename="{_skill_filename(row.data)}.SKILL.md"'},
    )


@router.get("/skills/{key}/export.zip")
def export_skill_zip(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Skill, key, s.owner_id)
    return Response(
        skills.export_archive(row.data),
        media_type="application/zip",
        headers={"Content-Disposition": f'attachment; filename="{_skill_filename(row.data)}.zip"'},
    )


@router.get("/skills/{key}/resources")
def skill_resource(key: str, path: str = "", ctx=Depends(context)):
    db, s = ctx
    own(db, Skill, key, s.owner_id)
    try:
        return skills.read_resource(db, s.owner_id, key, path)
    except ValueError as exc:
        raise HTTPException(404, str(exc)) from exc
