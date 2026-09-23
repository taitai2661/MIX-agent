"""Workspace/URL/text knowledge store with chunked portable search."""

from __future__ import annotations

import re

from sqlalchemy import select

from mix_agent.db.models import Knowledge, now, uid
from mix_agent.memory.service import SENSITIVE, terms

CHUNK_SIZE = 2000
CHUNK_OVERLAP = 400
MAX_CHUNKS = 100
MAX_CONTENT_CHARS = 200_000


def chunk_text(content: str, size: int = CHUNK_SIZE, overlap: int = CHUNK_OVERLAP):
    text = re.sub(r"\s+", " ", content or "").strip()
    if not text:
        return []
    chunks, start = [], 0
    while start < len(text):
        end = min(len(text), start + size)
        # Prefer a sentence boundary near the end of the window.
        boundary = max(text.rfind(marker, start, end) for marker in ("。", ".\n", "\n", ". "))
        if end < len(text) and boundary > start + size // 2:
            end = boundary + 1
        chunks.append(text[start:end].strip())
        if end >= len(text):
            break
        start = max(end - overlap, start + 1)
        if len(chunks) >= MAX_CHUNKS:
            break
    return [chunk for chunk in chunks if chunk]


def _validate(content: str):
    if not content or not content.strip():
        raise ValueError("Knowledge content is empty")
    if len(content) > MAX_CONTENT_CHARS:
        raise ValueError("Knowledge content exceeds 200000 characters")
    if SENSITIVE.search(content):
        raise ValueError("Knowledgeには認証情報・トークン・パスワードを保存できません")


def add(db, owner: str, content: str, *, title: str = "", source_type: str = "text",
        source_ref: str = "", memo: str = ""):
    _validate(content)
    text = re.sub(r"\s+", " ", content).strip()
    doc_id = uid()
    chunks = chunk_text(text)
    if not chunks:
        raise ValueError("Knowledge content is empty")
    stored_at = now()
    for index, chunk in enumerate(chunks):
        db.add(Knowledge(
            owner_id=owner,
            data={
                "doc_id": doc_id,
                "title": (title or source_ref or text[:80])[:500],
                "source_type": source_type,
                "source_ref": source_ref,
                "chunk_index": index,
                "chunk_count": len(chunks),
                "content": chunk,
                "memo": memo,
                "embedding_version": 0,
                "stored_at": stored_at.isoformat(),
            },
        ))
    db.flush()
    return {"id": doc_id, "title": (title or source_ref or text[:80])[:500],
            "chunks": len(chunks), "source_type": source_type, "source_ref": source_ref}


def _score(query_terms: set[str], content: str, title: str, memo: str):
    doc_terms = terms(f"{title} {content} {memo}")
    if not query_terms:
        return 0.0
    overlap = len(query_terms & doc_terms) / len(query_terms)
    # Title matches are a strong intent signal.
    title_terms = terms(title)
    title_bonus = 0.15 * (len(query_terms & title_terms) / len(query_terms)) if title_terms else 0.0
    return overlap + title_bonus


def search(db, owner: str, query: str = "", top_k: int = 5):
    top_k = max(1, min(20, int(top_k or 5)))
    query_terms = terms(query)
    rows = list(db.scalars(
        select(Knowledge).where(Knowledge.owner_id == owner).order_by(Knowledge.created_at.desc()).limit(2000)
    ))
    scored = []
    for row in rows:
        data = row.data or {}
        value = _score(query_terms, data.get("content", ""), data.get("title", ""), data.get("memo", ""))
        if query_terms and value <= 0:
            continue
        scored.append((value, row))
    scored.sort(key=lambda item: (item[0], item[1].created_at), reverse=True)
    results = []
    for value, row in scored[:top_k]:
        data = dict(row.data or {})
        excerpt = data.get("content", "")
        results.append({
            "chunk_id": row.id,
            "id": data.get("doc_id", row.id),
            "title": data.get("title", ""),
            "source_type": data.get("source_type", ""),
            "source_ref": data.get("source_ref", ""),
            "chunk_index": data.get("chunk_index", 0),
            "chunk_count": data.get("chunk_count", 1),
            "excerpt": excerpt[:2000],
            "memo": data.get("memo", ""),
            "relevance": round(min(1.0, value), 4),
        })
    return results


def list_documents(db, owner: str, limit: int = 100):
    rows = list(db.scalars(
        select(Knowledge).where(Knowledge.owner_id == owner).order_by(Knowledge.created_at.desc()).limit(min(limit, 500))
    ))
    documents: dict[str, dict] = {}
    for row in rows:
        data = row.data or {}
        doc_id = data.get("doc_id", row.id)
        if doc_id not in documents:
            documents[doc_id] = {
                "id": doc_id,
                "title": data.get("title", ""),
                "source_type": data.get("source_type", ""),
                "source_ref": data.get("source_ref", ""),
                "chunks": data.get("chunk_count", 1),
                "memo": data.get("memo", ""),
                "created_at": row.created_at.isoformat() if row.created_at else None,
            }
    return list(documents.values())[:limit]


def delete(db, owner: str, doc_id: str):
    rows = list(db.scalars(select(Knowledge).where(Knowledge.owner_id == owner)))
    matched = [row for row in rows if (row.data or {}).get("doc_id") == doc_id or row.id == doc_id]
    if not matched:
        raise ValueError("Knowledge document was not found")
    for row in matched:
        db.delete(row)
    db.flush()
    return {"id": doc_id, "deleted": True, "chunks": len(matched)}
