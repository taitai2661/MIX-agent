from sqlalchemy import select
from mix_agent.db.models import Skill, SkillRevision


def search(db, owner, query="", ids=None):
    rows = db.scalars(select(Skill).where(Skill.owner_id == owner).order_by(Skill.created_at.desc()))
    wanted = set(ids or [])
    result = []
    for row in rows:
        data = row.data
        if data.get("deleted") or not data.get("enabled", True):
            continue
        if wanted and row.id not in wanted:
            continue
        haystack = " ".join(str(data.get(k, "")) for k in ("name", "description", "content"))
        if query.casefold() in haystack.casefold():
            result.append({"id": row.id, **data})
    return result[:20]


def change(db, owner, name=None, description=None, content=None, skill_id=None, delete=False, enabled=True, source_run=None):
    if skill_id:
        row = db.get(Skill, skill_id)
        if not row or row.owner_id != owner:
            raise ValueError("Skill not found")
        db.add(SkillRevision(owner_id=owner, data={"skill_id": row.id, "previous": row.data}))
        row.data = {**row.data, **({"name": name} if name is not None else {}), **({"description": description} if description is not None else {}), **({"content": content} if content is not None else {}), "enabled": enabled, "deleted": delete, "source_run": source_run or row.data.get("source_run")}
    else:
        row = Skill(owner_id=owner, data={"name": name, "description": description or "", "content": content, "enabled": enabled, "deleted": False, "source_run": source_run})
        db.add(row)
    db.flush()
    return {"id": row.id, **row.data}
