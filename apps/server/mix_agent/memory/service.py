from sqlalchemy import select
from mix_agent.db.models import Memory, MemoryRevision


def accessible(row, owner, scopes):
    return (
        row
        and row.owner_id == owner
        and row.data.get("scope", "user") in scopes
        and not row.data.get("deleted")
    )


def search(db, owner, query="", scopes=None):
    scopes = ["user"] if scopes is None else scopes
    if not scopes:
        return []
    rows = db.scalars(
        select(Memory)
        .where(Memory.owner_id == owner, Memory.data["content"].as_string().icontains(query, autoescape=True))
        .limit(100)
    )
    return [{"id": r.id, **r.data} for r in rows if accessible(r, owner, scopes)][:30]


def change(db, owner, content=None, memory_id=None, delete=False, scope="user", source_run=None, scopes=None):
    if memory_id:
        row = db.get(Memory, memory_id)
        if not accessible(row, owner, scopes or ["user"]):
            raise ValueError("Memory not found in permitted scope")
        db.add(MemoryRevision(owner_id=owner, data={"memory_id": row.id, "previous": row.data}))
        row.data = {
            **row.data,
            "content": content if content is not None else row.data["content"],
            "deleted": delete,
            "source_run": source_run,
        }
    else:
        row = Memory(
            owner_id=owner,
            data={"content": content, "scope": scope, "source_run": source_run, "deleted": False},
        )
        db.add(row)
    db.flush()
    return {"id": row.id, **row.data}
