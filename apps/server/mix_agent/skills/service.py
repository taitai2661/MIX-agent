from sqlalchemy import select

from mix_agent.db.models import Skill, SkillRevision
from mix_agent.skills import package


def _context_view(data: dict) -> dict:
    """Shrink a stored skill for model context: bundled files as paths only."""
    view = {key: value for key, value in data.items() if key != "files"}
    if data.get("files"):
        view["files"] = sorted(data["files"].keys())
    return view


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
        haystack = " ".join(
            [str(data.get(k, "")) for k in ("name", "description", "content")]
            + sorted((data.get("files") or {}).keys())
        )
        if query.casefold() in haystack.casefold():
            result.append({"id": row.id, **_context_view(data)})
    return result[:20]


def _validate_identity(name, description):
    """Apply the SKILL.md frontmatter rule to the free-form write paths.

    ``POST /skills``, ``PATCH`` and the ``skill_add`` / ``skill_update`` tools
    take name and description directly rather than through a ``SKILL.md``, so
    without this they bypass the rule the import paths enforce.
    """
    errors = package.validate_meta({"name": name, "description": description})
    if errors:
        raise ValueError("; ".join(errors))


def change(db, owner, name=None, description=None, content=None, skill_id=None, delete=False, enabled=True, source_run=None):
    if skill_id:
        row = db.get(Skill, skill_id)
        if not row or row.owner_id != owner:
            raise ValueError("Skill not found")
        # Validate only the fields this call actually changes: a patch that
        # leaves the name alone (for instance toggling `enabled`) must keep
        # working for a row stored before the rule existed, while a new name
        # or description still has to conform.
        name_changed = name is not None and name != row.data.get("name")
        description_changed = description is not None and description != row.data.get("description", "")
        if name_changed or description_changed:
            _validate_identity(
                name if name_changed else row.data.get("name"),
                description if description_changed else row.data.get("description", ""),
            )
        db.add(SkillRevision(owner_id=owner, data={"skill_id": row.id, "previous": row.data}))
        row.data = {**row.data, **({"name": name} if name is not None else {}), **({"description": description} if description is not None else {}), **({"content": content} if content is not None else {}), "enabled": enabled, "deleted": delete, "source_run": source_run or row.data.get("source_run")}
    else:
        _validate_identity(name, description or "")
        row = Skill(owner_id=owner, data={"name": name, "description": description or "", "content": content, "enabled": enabled, "deleted": False, "source_run": source_run})
        db.add(row)
    db.flush()
    return {"id": row.id, **row.data}


def _find_by_slug(db, owner, slug):
    rows = db.scalars(select(Skill).where(Skill.owner_id == owner))
    for row in rows:
        if row.data.get("source_slug") == slug:
            return row
    return None


def find(db, owner, ref):
    """Resolve a skill by id, spec name, or display name."""
    if ref:
        row = db.get(Skill, ref)
        if row and row.owner_id == owner:
            return row
    rows = db.scalars(select(Skill).where(Skill.owner_id == owner))
    for row in rows:
        if row.data.get("deleted") or not row.data.get("enabled", True):
            continue
        if ref in (row.data.get("name"), row.data.get("source_slug")):
            return row
    return None


def import_package(db, owner, parsed, source="upload", source_path=None, source_run=None):
    """Create or update a skill from a parsed Agent Skills package."""
    slug = parsed.get("source_slug") or parsed["name"]
    existing = _find_by_slug(db, owner, slug)
    data = {
        "name": parsed["name"],
        "description": parsed.get("description", ""),
        "content": parsed.get("content", ""),
        "enabled": True,
        "deleted": False,
        "format": "agent-skill",
        "license": parsed.get("license", ""),
        "compatibility": parsed.get("compatibility", ""),
        "metadata": parsed.get("metadata") or {},
        "allowed_tools": parsed.get("allowed_tools") or [],
        "files": parsed.get("files") or {},
        "source": source,
        "source_slug": slug,
        "source_path": source_path,
        "source_run": source_run or (existing.data.get("source_run") if existing else None),
    }
    if existing:
        db.add(SkillRevision(owner_id=owner, data={"skill_id": existing.id, "previous": existing.data}))
        existing.data = data
        row = existing
    else:
        row = Skill(owner_id=owner, data=data)
        db.add(row)
    db.flush()
    return {"id": row.id, **row.data}


def export_markdown(data: dict) -> str:
    return package.render_skill_md(data)


def export_archive(data: dict) -> bytes:
    return package.bundle_archive(data)


def read_resource(db, owner, skill_id, path=""):
    row = db.get(Skill, skill_id)
    if not row or row.owner_id != owner:
        raise ValueError("Skill not found")
    files = row.data.get("files") or {}
    if not path:
        return {
            "files": [
                {"path": name, "size": entry.get("size", 0), "binary": bool(entry.get("binary"))}
                for name, entry in sorted(files.items())
            ]
        }
    try:
        safe = package.safe_relative_path(path)
    except package.SkillFormatError:
        raise ValueError("Invalid resource path") from None
    entry = files.get(safe)
    if entry is None:
        raise ValueError("Resource not found")
    return {
        "path": safe,
        "size": entry.get("size", 0),
        "binary": bool(entry.get("binary")),
        "content": entry.get("content", ""),
    }
