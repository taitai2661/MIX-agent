import re
from datetime import UTC, datetime

from sqlalchemy import select
from mix_agent.db.models import Memory, MemoryRevision


SENSITIVE = re.compile(r"(?:api[_ -]?key|password|secret|token|authorization)\s*[:=]", re.IGNORECASE)
STOP_WORDS = frozenset({"これ", "それ", "ため", "です", "ます", "する", "した", "して", "with", "that", "this", "from", "your"})


def terms(value):
    """Small, dependency-free lexical recall that works for Japanese and Latin text."""
    text = re.sub(r"\s+", " ", (value or "").casefold()).strip()
    latin = re.findall(r"[\w-]{2,}", text, flags=re.UNICODE)
    japanese = re.findall(r"[\u3040-\u30ff\u3400-\u9fff]{2,}", text)
    grams = [part[i : i + 2] for part in japanese for i in range(len(part) - 1)]
    return {part for part in [*latin, *grams] if part not in STOP_WORDS}


def classify(content):
    lowered = content.casefold()
    if any(word in lowered for word in ("好む", "希望", "prefer", "always", "日本語", "tone")):
        return "preference"
    if any(word in lowered for word in ("決定", "採用", "decision", "choose", "方針")):
        return "decision"
    if any(word in lowered for word in ("名前", "role", "職業", "プロフィール")):
        return "profile"
    return "fact"


def score(row, query_terms):
    data = row.data
    overlap = len(query_terms & terms(data.get("content", "")))
    # Pinned and explicit preferences remain available even on terse follow-ups.
    base = int(data.get("importance", 2)) * 4 + (20 if data.get("pinned") else 0)
    if query_terms and not overlap and not data.get("pinned"):
        return -1
    created = row.created_at.replace(tzinfo=UTC) if row.created_at.tzinfo is None else row.created_at
    age_days = max(0, (datetime.now(UTC) - created).days)
    return base + overlap * 15 + max(0, 3 - age_days // 90)


def explicit_candidate(content):
    """Extract only an unambiguous user request to remember; never infer one."""
    text = (content or "").strip()
    patterns = (
        r"^(?:覚えて|記憶して|remember)\s*[:：]\s*(.+)$",
        r"^(.{2,500}?)\s*(?:を)?(?:覚えておいて|記憶しておいて|remember this)\s*[。.!！]?$",
    )
    for pattern in patterns:
        match = re.match(pattern, text, re.IGNORECASE)
        if match:
            candidate = match.group(1).strip(" \t\n。.!！")
            if candidate and not SENSITIVE.search(candidate):
                return candidate
    return None


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
    rows = db.scalars(select(Memory).where(Memory.owner_id == owner)).all()
    query_terms = terms(query)
    ranked = [(score(row, query_terms), row) for row in rows if accessible(row, owner, scopes)]
    ranked = [item for item in ranked if item[0] >= 0]
    ranked.sort(key=lambda item: (item[0], item[1].created_at), reverse=True)
    return [{"id": row.id, **row.data} for _, row in ranked[:30]]


def change(db, owner, content=None, memory_id=None, delete=False, scope="user", source_run=None, scopes=None, *, importance=2, category=None, pinned=False):
    if content is not None and SENSITIVE.search(content):
        raise ValueError("Memoryには認証情報・トークン・パスワードを保存できません")
    if content is not None:
        content = re.sub(r"\s+", " ", content).strip()
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
            "importance": importance if content is not None else row.data.get("importance", 2),
            "category": category or row.data.get("category") or classify(row.data["content"]),
            "pinned": pinned if content is not None else row.data.get("pinned", False),
        }
    else:
        # Avoid silently accumulating identical facts when an agent remembers it twice.
        normalized = content.casefold()
        existing = next((r for r in db.scalars(select(Memory).where(Memory.owner_id == owner)) if not r.data.get("deleted") and r.data.get("scope", "user") == scope and r.data.get("content", "").casefold() == normalized), None)
        if existing:
            return {"id": existing.id, **existing.data, "deduplicated": True}
        row = Memory(
            owner_id=owner,
            data={"content": content, "scope": scope, "source_run": source_run, "deleted": False, "importance": importance, "category": category or classify(content), "pinned": pinned},
        )
        db.add(row)
    db.flush()
    return {"id": row.id, **row.data}
