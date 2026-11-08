"""Memory service: create/update/delete one flat Memory row.

The taxonomy constants and row helpers live in ``common.py``; recall, §4
links and §3 lifecycle transitions have their own modules. This file keeps the
write path and re-exports the old public surface so existing callers
(``memory.search``, ``memory.link``, …) keep working unchanged.
"""

from __future__ import annotations

import re

from sqlalchemy import select

from mix_agent.db.models import Memory, MemoryRevision, now
from mix_agent.memory.types import LIFECYCLE_CANDIDATE, LIFECYCLE_ESTABLISHED
from mix_agent.memory.common import (
    AUTOMATIC_SETTING_KEYS,
    AUTO_OVERRIDES_IGNORED,
    CANDIDATE_LIMIT,
    DEFAULTS,
    DECAY_FLOOR,
    DECAY_HALF_LIFE_DAYS,
    DECAY_MIN_DAYS,
    DEFAULT_CONFIDENCE,
    DEFAULT_LIFECYCLE,
    DEFAULT_SALIENCE,
    DEFAULT_STRENGTH,
    ITEM_CHAR_LIMIT,
    LEXICAL_WEIGHT_WITH_VECTOR,
    LEXICAL_WEIGHT_WITHOUT_VECTOR,
    LINK_DEFAULT_WEIGHT,
    LINK_DEGREE_LIMIT,
    LINK_EXPANSION_LIMIT,
    LINK_NEIGHBOR_MIN_SIMILARITY,
    LINK_NEIGHBOR_SETTING,
    LINK_NEIGHBOR_TOP_K,
    LINK_SUGGEST_LIMIT,
    MIN_SCORE,
    RECENCY_WEIGHT_WITHOUT_VECTOR,
    RECENCY_WEIGHT_WITH_VECTOR,
    RESULT_LIMIT_DEFAULT,
    RESULT_LIMIT_MAX,
    ROLE_FACT,
    SCOPE_MISMATCH_FACTOR,
    SCOPE_MATCH_FACTOR,
    SCOPE_USER,
    SENSITIVE,
    SOURCE_AGENT,
    VECTOR_WEIGHT_WITH_VECTOR,
    _content,
    _event,
    _normalize,
    _trace_data,
    clamp,
    explicit_candidate,
    explicit_forget_candidate,
    is_pinned,
    lifecycle_value,
    number,
    redact_sensitive,
    retrievable,
    role_value,
    scope_value,
    scope_weight,
    source_kind_value,
    terms,
)
from mix_agent.memory.lifecycle import (
    decay,
    reinforce_row,
    resolve_conflict,
    restore,
    restore_deleted,
)
from mix_agent.memory.links import (
    _detach_pointers,
    associations_for,
    embedding_neighbor_links,
    link,
    link_counts,
    suggest_links,
    sync_links,
)
from mix_agent.memory.recall import list_traces, rank, search

__all__ = [
    "AUTOMATIC_SETTING_KEYS",
    "AUTO_OVERRIDES_IGNORED",
    "CANDIDATE_LIMIT",
    "DEFAULTS",
    "DECAY_FLOOR",
    "DECAY_HALF_LIFE_DAYS",
    "DECAY_MIN_DAYS",
    "DEFAULT_CONFIDENCE",
    "DEFAULT_LIFECYCLE",
    "DEFAULT_SALIENCE",
    "DEFAULT_STRENGTH",
    "ITEM_CHAR_LIMIT",
    "LEXICAL_WEIGHT_WITH_VECTOR",
    "LEXICAL_WEIGHT_WITHOUT_VECTOR",
    "LIFECYCLE_CANDIDATE",
    "LIFECYCLE_ESTABLISHED",
    "LINK_DEFAULT_WEIGHT",
    "LINK_DEGREE_LIMIT",
    "LINK_EXPANSION_LIMIT",
    "LINK_NEIGHBOR_MIN_SIMILARITY",
    "LINK_NEIGHBOR_SETTING",
    "LINK_NEIGHBOR_TOP_K",
    "LINK_SUGGEST_LIMIT",
    "MIN_SCORE",
    "RECENCY_WEIGHT_WITHOUT_VECTOR",
    "RECENCY_WEIGHT_WITH_VECTOR",
    "RESULT_LIMIT_DEFAULT",
    "RESULT_LIMIT_MAX",
    "ROLE_FACT",
    "SCOPE_MISMATCH_FACTOR",
    "SCOPE_MATCH_FACTOR",
    "SCOPE_USER",
    "SENSITIVE",
    "SOURCE_AGENT",
    "VECTOR_WEIGHT_WITH_VECTOR",
    "associations_for",
    "change",
    "clamp",
    "decay",
    "embedding_neighbor_links",
    "explicit_candidate",
    "explicit_forget_candidate",
    "is_pinned",
    "lifecycle_value",
    "link",
    "link_counts",
    "list_traces",
    "number",
    "rank",
    "redact_sensitive",
    "reinforce_row",
    "resolve_conflict",
    "restore",
    "restore_deleted",
    "retrievable",
    "role_value",
    "scope_value",
    "scope_weight",
    "search",
    "source_kind_value",
    "suggest_links",
    "sync_links",
    "terms",
]


def change(db, owner, content=None, memory_id=None, delete=False, source_run=None, **kwargs):
    """Create / update / delete one flat Memory row, keeping spec §1 in step.

    Unknown taxonomy kwargs are still ignored, but the §1 columns that *do*
    exist now take their value: each is validated against §2 first, so an
    unusable string falls back to the row default instead of being persisted
    as something no renderer understands. ``role`` additionally keeps its
    ``data.role`` mirror (§6's extractor wrote it there before 0024 and old
    readers still look for it); the column is what §1 calls the truth.

    Spec §3 initial lifecycle: explicit human input — ``source_run=
    "explicit-user-request"``, ``pinned``, or a caller that says so — is
    stored ``established``; automatic input (the §6 extractor, the agent's
    ``memory_remember``) is stored ``candidate`` until a transition confirms
    it. Both are §2 ``retrievable_lifecycles``, so this changes confidence,
    not findability.
    """
    pinned = bool(kwargs.get("pinned", False))
    role = role_value(kwargs.get("role"))
    scope = scope_value(kwargs.get("scope"))
    lifecycle = lifecycle_value(kwargs.get("lifecycle_state"))
    source_kind = source_kind_value(kwargs.get("source_kind"))
    confidence = number(kwargs.get("confidence"), None) if kwargs.get("confidence") is not None else None
    salience = number(kwargs.get("salience"), None) if kwargs.get("salience") is not None else None
    strength = number(kwargs.get("strength"), None) if kwargs.get("strength") is not None else None
    if content is not None:
        if SENSITIVE.search(content):
            raise ValueError("Memoryには認証情報・トークン・パスワードを保存できません")
        content = re.sub(r"\s+", " ", content).strip()
    if memory_id:
        row = db.get(Memory, memory_id)
        if not row or row.owner_id != owner:
            raise ValueError("Memory not found")
        db.add(
            MemoryRevision(
                owner_id=owner, data={"memory_id": row.id, "previous": {"id": row.id, **_trace_data(row)}}
            )
        )
        if delete:
            # §4: the edges go with the row (FK cascade), and every row that
            # pointed *at* it loses that pointer in the same transaction —
            # `superseded_by_id` naming a deleted memory is exactly the
            # half-updated state §4 forbids.
            _detach_pointers(db, owner, memory_id)
            db.delete(row)
            db.flush()
            _event(db, owner, source_run, "DELETE", memory_id, None, None, "explicit delete")
            return {"id": memory_id, "deleted": True}
        if content:
            row.content = content[:10000]
            data = dict(row.data or {})
            # The text changed, so the stored vector now describes different
            # words. Drop it and let the next write/search re-embed rather than
            # rank this row with a vector for content it no longer holds.
            data.pop("embedding", None)
            data.pop("embedding_model", None)
            data.update(
                {
                    "content": row.content,
                    "pinned": pinned or data.get("pinned", False),
                    "source_run": source_run or data.get("source_run"),
                    "source": data.get("source", "user"),
                }
            )
            if role:
                data["role"] = role
            if strength is not None:
                data["strength"] = strength
            row.data = data
            row.updated_at = now()
        # §1 taxonomy on update: only what the caller supplied and §2 allows.
        if role:
            row.role = role
        if scope:
            row.scope = scope
        if lifecycle:
            row.lifecycle_state = lifecycle
        if source_kind:
            row.source_kind = source_kind
        if confidence is not None:
            row.confidence = confidence
        if salience is not None:
            row.salience = salience
        db.flush()
        return {"id": row.id, **_trace_data(row)}
    # create path
    if not content:
        raise ValueError("Memory content is required")
    if len(content) > 10000:
        content = content[:10000]
    normalized = _normalize(content)
    existing = None
    for row in db.scalars(
        select(Memory).where(Memory.owner_id == owner).order_by(Memory.updated_at.desc()).limit(200)
    ):
        if _normalize(_content(row)) == normalized:
            existing = row
            break
    if existing:
        # A repeated fact reinforces the row it already has (§3): the same
        # words arriving again are evidence, not a second memory. The event
        # below says REINFORCE, so the row itself has to be reinforced —
        # bumping only `updated_at` recorded the activation without any of
        # its effect.
        reinforce_row(existing)
        if role:
            # A repeated fact is a chance to attach the role it was stored
            # without the first time; the row itself is never duplicated.
            existing.data = {**(existing.data or {}), "role": role}
            existing.role = role
        db.flush()
        _event(db, owner, source_run, "REINFORCE", existing.id, None, None, "duplicate")
        return {"id": existing.id, **_trace_data(existing), "deduplicated": True}
    source = "user" if (source_run == "explicit-user-request" or pinned) else "auto"
    row = Memory(
        owner_id=owner,
        content=content,
        updated_at=now(),
        lifecycle_state=lifecycle or (LIFECYCLE_ESTABLISHED if source == "user" else LIFECYCLE_CANDIDATE),
        role=role or ROLE_FACT,
        scope=scope or SCOPE_USER,
        source_kind=source_kind or ("user" if source == "user" else SOURCE_AGENT),
        confidence=DEFAULT_CONFIDENCE if confidence is None else confidence,
        salience=(1.0 if pinned else DEFAULT_SALIENCE) if salience is None else salience,
        data={
            "content": content,
            "pinned": pinned,
            "source": source,
            "source_run": source_run,
            **({"role": role} if role else {}),
            **({"strength": strength} if strength is not None else {}),
        },
    )
    db.add(row)
    db.flush()
    # mirror content into data for old readers that only look at JSONB
    row.data = {**(row.data or {}), "content": content}
    db.flush()
    _event(db, owner, source_run, "CREATE", row.id, None, None, "explicit" if source == "user" else "auto")
    db.flush()
    return {"id": row.id, **_trace_data(row)}
