"""ChatGPT-style flat memory: one fact per row, text match + recency.

No roles / scopes / lifecycle / verification / associations / feature vectors.
API kept backward-compatible where cheap (``**kwargs`` ignored) so callers
like engine/routes/tools keep working while the stored shape is flat:
``Memory.content`` + ``data={pinned, source, source_run}``.
"""

from __future__ import annotations

import re
from datetime import UTC, datetime

from sqlalchemy import func, or_, select

from mix_agent.db.models import Memory, MemoryActionEvent, MemoryRevision, now

SENSITIVE = re.compile(
    r"(?:api[_ -]?key|password|passwd|secret|access[_ -]?token|session[_ -]?token|authorization|private[_ -]?key|cookie|認証コード|パスワード)\s*[:=：]",
    re.IGNORECASE,
)
SENSITIVE_VALUE = re.compile(
    r"(?:api[_ -]?key|password|passwd|secret|access[_ -]?token|session[_ -]?token|authorization|private[_ -]?key|cookie|認証コード|パスワード)\s*[:=：]\s*[^\s,;]+",
    re.IGNORECASE,
)
STOP_WORDS = frozenset({"これ", "それ", "ため", "です", "ます", "する", "した", "して", "with", "that", "this", "from", "your"})
DEFAULTS = {"result_limit": 8}
# Everything the old taxonomy accepted is now ignored (kept only so old
# callers/tests don't crash on unexpected kwargs).
AUTOMATIC_SETTING_KEYS: tuple[str, ...] = ()
AUTO_OVERRIDES_IGNORED: tuple[str, ...] = ()


def clamp(value, low=0.0, high=1.0):
    return max(low, min(high, float(value)))


def terms(value):
    text = re.sub(r"\s+", " ", (value or "").casefold()).strip()
    latin = re.findall(r"[\w-]{2,}", text, flags=re.UNICODE)
    japanese = re.findall(r"[\u3040-\u30ff\u3400-\u9fff]{2,}", text)
    grams = [part[i : i + 2] for part in japanese for i in range(len(part) - 1)]
    return {part[:160] for part in [*latin, *grams] if part not in STOP_WORDS}


def explicit_candidate(content):
    text = (content or "").strip()
    for pattern in (r"^(?:覚えて|記憶して|remember)\s*[:：]\s*(.+)$", r"^(.{2,500}?)\s*(?:を)?(?:覚えておいて|記憶しておいて|remember this)\s*[。.!！]?$"):
        match = re.match(pattern, text, re.IGNORECASE)
        if match:
            candidate = match.group(1).strip(" \t\n。.!！")
            if candidate and not SENSITIVE.search(candidate):
                return candidate
    return None


def explicit_forget_candidate(content):
    text = (content or "").strip()
    for pattern in (r"^(?:忘れて|記憶から消して)\s*[:：]\s*(.+)$", r"^(.{2,500}?)\s*(?:を)?(?:忘れて|記憶から消して)\s*[。.!！]?$"):
        match = re.match(pattern, text, re.IGNORECASE)
        if match:
            return match.group(1).strip(" \t\n。.!！")
    return None


def redact_sensitive(content):
    value = re.sub(r"-----BEGIN [^-]+PRIVATE KEY-----.*?-----END [^-]+PRIVATE KEY-----", "[REDACTED SECRET]", content or "", flags=re.DOTALL | re.IGNORECASE)
    return SENSITIVE_VALUE.sub("[REDACTED SECRET]", value)


def _content(row: Memory) -> str:
    if getattr(row, "content", None):
        return row.content
    return (row.data or {}).get("content", "") or (row.data or {}).get("gist", "")


def _trace_data(row: Memory) -> dict:
    """Flat public shape (ChatGPT-like). Extra legacy keys kept for readers."""
    data = dict(row.data or {})
    content = _content(row)
    return {
        "content": content,
        "gist": content[:300],
        "pinned": bool(data.get("pinned")),
        "source": data.get("source", "user"),
        "source_run": data.get("source_run"),
        "created_at": row.created_at.isoformat() if row.created_at else None,
        "updated_at": row.updated_at.isoformat() if getattr(row, "updated_at", None) else None,
        # legacy compat (old UI/tests read these; now constants)
        "lifecycle_state": "established",
        "role": "fact",
        "scope": "user",
        "strength": 0.6,
        "confidence": 0.7,
        "salience": 0.5,
    }


def _event(db, owner, run_id, action, memory_id, before, after, reason, candidate_index=0):
    db.add(MemoryActionEvent(owner_id=owner, run_id=run_id if run_id and run_id != "explicit-user-request" else None, candidate_index=candidate_index, data={"action": action, "memory_id": memory_id, "reason": reason}))


def _normalize(text: str) -> str:
    return re.sub(r"\s+", " ", (text or "").strip().casefold())


def change(db, owner, content=None, memory_id=None, delete=False, source_run=None, **kwargs):
    """ChatGPT-like CRUD. Unknown taxonomy kwargs are ignored."""
    pinned = bool(kwargs.get("pinned", False))
    if content is not None:
        if SENSITIVE.search(content):
            raise ValueError("Memoryには認証情報・トークン・パスワードを保存できません")
        content = re.sub(r"\s+", " ", content).strip()
    if memory_id:
        row = db.get(Memory, memory_id)
        if not row or row.owner_id != owner:
            raise ValueError("Memory not found")
        db.add(MemoryRevision(owner_id=owner, data={"memory_id": row.id, "previous": {"id": row.id, **_trace_data(row)}}))
        if delete:
            db.delete(row)
            db.flush()
            _event(db, owner, source_run, "DELETE", memory_id, None, None, "explicit delete")
            return {"id": memory_id, "deleted": True}
        if content:
            row.content = content[:10000]
            data = dict(row.data or {})
            data.update({"content": row.content, "pinned": pinned or data.get("pinned", False), "source_run": source_run or data.get("source_run"), "source": data.get("source", "user")})
            row.data = data
            row.updated_at = now()
        db.flush()
        return {"id": row.id, **_trace_data(row)}
    # create path
    if not content:
        raise ValueError("Memory content is required")
    if len(content) > 10000:
        content = content[:10000]
    normalized = _normalize(content)
    existing = None
    for row in db.scalars(select(Memory).where(Memory.owner_id == owner).order_by(Memory.updated_at.desc()).limit(200)):
        if _normalize(_content(row)) == normalized:
            existing = row
            break
    if existing:
        existing.updated_at = now()
        db.flush()
        _event(db, owner, source_run, "REINFORCE", existing.id, None, None, "duplicate")
        return {"id": existing.id, **_trace_data(existing), "deduplicated": True}
    source = "user" if (source_run == "explicit-user-request" or pinned) else "auto"
    row = Memory(
        owner_id=owner,
        content=content,
        updated_at=now(),
        data={"content": content, "pinned": pinned, "source": source, "source_run": source_run},
    )
    db.add(row)
    db.flush()
    # mirror content into data for old readers that only look at JSONB
    row.data = {**(row.data or {}), "content": content}
    db.flush()
    _event(db, owner, source_run, "CREATE", row.id, None, None, "explicit" if source == "user" else "auto")
    db.flush()
    return {"id": row.id, **_trace_data(row)}


def _score(query_terms: set[str], content: str, updated_at) -> tuple[float, dict]:
    row_terms = terms(content)
    overlap = len(query_terms & row_terms) / max(1, len(query_terms)) if query_terms else 0.0
    try:
        ts = updated_at
        if ts is not None and ts.tzinfo is None:
            ts = ts.replace(tzinfo=UTC)
        age_days = max(0.0, (datetime.now(UTC) - ts).total_seconds() / 86400) if ts else 365.0
    except Exception:
        age_days = 365.0
    recency = 1 / (1 + age_days / 180)
    # ChatGPT-like: lexical match dominates, recency breaks ties.
    value = 0.85 * overlap + 0.15 * recency
    return value, {"lexical": round(overlap, 4), "recency": round(recency, 4)}


def search(db, owner, query="", scopes=None, *, settings=None, debug=False, activate=False, **kwargs):
    limit = 8
    try:
        limit = max(1, min(30, int((settings or {}).get("result_limit", 8))))
    except Exception:
        pass
    query = (query or "").strip()
    query_terms = terms(query)
    rows = list(db.scalars(select(Memory).where(Memory.owner_id == owner).order_by(Memory.updated_at.desc()).limit(200)))
    if not query_terms:
        ranked = rows[:limit]
        result = [{"id": r.id, **_trace_data(r), "relevance": 0.0, "selection_reason": "recent"} for r in ranked]
    else:
        scored = []
        for row in rows:
            value, breakdown = _score(query_terms, _content(row), getattr(row, "updated_at", None) or row.created_at)
            if value >= 0.10 or query_terms & terms(_content(row)):
                scored.append((value, row, breakdown))
        # unrelated queries return nothing (avoid polluting context)
        if not scored or max(s[0] for s in scored) < 0.10:
            scored = []
        scored.sort(key=lambda x: (x[0], getattr(x[1], "updated_at", None) or x[1].created_at), reverse=True)
        result = [{"id": r.id, **_trace_data(r), "relevance": round(v, 4), "selection_reason": "語彙が一致" if b["lexical"] >= 0.4 else "文脈から関連"} for v, r, b in scored[:limit]]
    if debug:
        return {"memories": result, "debug": {"considered": len(rows), "kept": len(result), "stop_reason": "flat", "elapsed_ms": 0.0, "auto": {}, "trace": [], "seed_ids": [r["id"] for r in result], "association_expansion": [], "budget_exhausted": False, "honored_settings": [], "ignored_settings": [], "weights": {"lexical": 0.85, "recency": 0.15}, "lexical": 0.85}}
    return result


def list_traces(db, owner, query="", state=None, limit=200):
    rows = list(db.scalars(select(Memory).where(Memory.owner_id == owner).order_by(Memory.updated_at.desc()).limit(min(limit, 500))))
    qterms = terms(query)
    if qterms:
        rows = [r for r in rows if qterms & terms(_content(r))]
    return [{"id": r.id, **_trace_data(r)} for r in rows]


def associations_for(db, owner, memory_id, limit=20):
    return []


def restore(db, owner, row, previous):
    prev_content = (previous or {}).get("content", "") or (previous or {}).get("gist", "")
    if prev_content:
        row.content = str(prev_content)[:10000]
    data = dict(row.data or {})
    data["content"] = row.content
    if isinstance(previous, dict) and isinstance(previous.get("pinned"), bool):
        data["pinned"] = previous["pinned"]
    row.data = data
    row.updated_at = now()
    db.flush()
    return {"id": row.id, **_trace_data(row)}


def decay(db, owner, limit=200):
    return 0
