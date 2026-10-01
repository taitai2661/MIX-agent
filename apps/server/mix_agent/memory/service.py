"""Unified associative memory: context decides trace purpose at recall time."""

from __future__ import annotations

import hashlib
import logging
import math
import re
import time
from datetime import UTC, datetime

from sqlalchemy import delete, func, or_, select

from mix_agent.db.models import (
    Memory,
    MemoryActionEvent,
    MemoryAssociation,
    MemoryFeature,
    MemoryRevision,
    now,
)
from mix_agent.memory import types as mem_types

SENSITIVE = re.compile(
    r"(?:api[_ -]?key|password|passwd|secret|access[_ -]?token|session[_ -]?token|authorization|private[_ -]?key|cookie|認証コード|パスワード)\s*[:=：]",
    re.IGNORECASE,
)
SENSITIVE_VALUE = re.compile(
    r"(?:api[_ -]?key|password|passwd|secret|access[_ -]?token|session[_ -]?token|authorization|private[_ -]?key|cookie|認証コード|パスワード)\s*[:=：]\s*[^\s,;]+",
    re.IGNORECASE,
)
STOP_WORDS = frozenset({"これ", "それ", "ため", "です", "ます", "する", "した", "して", "with", "that", "this", "from", "your"})
LOGGER = logging.getLogger(__name__)
DEFAULTS = {"seed_limit": 24, "result_limit": 8}
AUTO_OVERRIDES_IGNORED = ("max_depth", "max_candidates", "min_association_weight", "activation_decay", "retrieval_budget_ms")
AUTOMATIC_SETTING_KEYS = ("memory_max_depth", "memory_max_candidates", "memory_retrieval_budget_ms", "memory_min_association_weight", "memory_activation_decay")
SEED_THRESHOLD = {
    "established": 0.18, "active": 0.18,
    "latent": 0.32, "candidate": 0.32,
}


def clamp(value, low=0.0, high=1.0):
    return max(low, min(high, float(value)))


def _auto_config(db, owner):
    """Derive retrieval shape from the owner's trace/graph scale, never from settings."""
    trace_count = int(db.scalar(select(func.count()).select_from(Memory).where(Memory.owner_id == owner, Memory.lifecycle_state.in_((mem_types.LIFECYCLE_ACTIVE, mem_types.LIFECYCLE_CANDIDATE, "established", "latent")))) or 0)
    edge_count = int(db.scalar(select(func.count()).select_from(MemoryAssociation).where(MemoryAssociation.owner_id == owner)) or 0)
    max_candidates = max(32, min(256, max(32, trace_count // 2)))
    min_weight = 0.20
    if edge_count:
        percentile = db.scalar(select(func.percentile_cont(0.25).within_group(MemoryAssociation.weight)).where(MemoryAssociation.owner_id == owner))
        min_weight = clamp(percentile if percentile is not None else 0.20, 0.10, 0.30)
    density = min(1.0, edge_count / max(1, trace_count * 5))
    return {
        "max_depth": 0 if edge_count == 0 else (1 if edge_count < 40 else (2 if edge_count < 600 else 3)),
        "max_candidates": max_candidates,
        "retrieval_budget_ms": max(60, min(400, 50 + max_candidates)),
        "min_association_weight": round(min_weight, 4),
        "activation_decay": round(clamp(0.60 - 0.15 * density, 0.40, 0.70), 4),
        "trace_count": trace_count,
        "edge_count": edge_count,
    }


def terms(value):
    text = re.sub(r"\s+", " ", (value or "").casefold()).strip()
    latin = re.findall(r"[\w-]{2,}", text, flags=re.UNICODE)
    japanese = re.findall(r"[\u3040-\u30ff\u3400-\u9fff]{2,}", text)
    grams = [part[i : i + 2] for part in japanese for i in range(len(part) - 1)]
    return {part[:160] for part in [*latin, *grams] if part not in STOP_WORDS}


def feature_vector(values, dimensions=64):
    vector = [0.0] * dimensions
    for value in values:
        digest = hashlib.blake2b(value.encode(), digest_size=8).digest()
        vector[int.from_bytes(digest[:4], "big") % dimensions] += -1.0 if digest[4] & 1 else 1.0
    norm = math.sqrt(sum(item * item for item in vector)) or 1.0
    return [round(item / norm, 6) for item in vector]


def cosine(left, right):
    return max(0.0, sum(a * b for a, b in zip(left, right))) if left and len(left) == len(right) else 0.0


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


def _state(row):
    if row.data.get("deleted"):
        return "deleted"
    return row.lifecycle_state or mem_types.LIFECYCLE_ACTIVE


def _trace_data(row):
    """Render a Memory row for the legacy associative-memory API.

    Preserves the historic ``lifecycle_state`` vocabulary (``established`` /
    ``latent`` / ``deleted``) so legacy tests and clients keep working.  New
    consumers should use :func:`mix_agent.memory.views.render` which exposes
    the role-based vocabulary.
    """
    data = dict(row.data or {})
    metadata = dict(data.get("metadata") or {})
    if data.get("category") and "legacy_category" not in metadata:
        metadata["legacy_category"] = data["category"]
    data.pop("category", None)
    data.pop("importance", None)
    lifecycle = _state(row)
    legacy_lifecycle = lifecycle
    if lifecycle == mem_types.LIFECYCLE_ACTIVE:
        legacy_lifecycle = "established"
    elif lifecycle == mem_types.LIFECYCLE_CANDIDATE:
        legacy_lifecycle = "latent"
    elif lifecycle == mem_types.LIFECYCLE_EXPIRED:
        legacy_lifecycle = "deleted"
    return {**data, "lifecycle_state": legacy_lifecycle, "strength": round(row.strength, 4), "confidence": round(row.confidence, 4), "salience": round(row.salience, 4), "activation_count": row.activation_count, "last_activated_at": row.last_activated_at.isoformat() if row.last_activated_at else None, "last_reinforced_at": row.last_reinforced_at.isoformat() if row.last_reinforced_at else None, "metadata": metadata, "importance": max(1, min(5, round(row.salience * 5))), "pinned": bool(data.get("pinned"))}


def _sync_features(db, row):
    db.execute(delete(MemoryFeature).where(MemoryFeature.memory_id == row.id))
    data = dict(row.data or {})
    groups = {
        "term": terms(" ".join(filter(None, (data.get("content", ""), data.get("gist", ""))))),
        "entity": {str(x).casefold()[:160] for x in data.get("entities", [])},
        "concept": {str(x).casefold()[:160] for x in data.get("concepts", [])},
    }
    data["feature_vector"] = feature_vector(set().union(*groups.values()))
    row.data = data
    for kind, values in groups.items():
        for value in values:
            db.add(MemoryFeature(owner_id=row.owner_id, memory_id=row.id, kind=kind, value=value, weight=1.0))


def _candidate_rows(db, owner, query_terms, limit):
    ids = []
    if query_terms:
        ids = list(db.scalars(select(MemoryFeature.memory_id).where(MemoryFeature.owner_id == owner, MemoryFeature.value.in_(list(query_terms)[:128])).group_by(MemoryFeature.memory_id).order_by(func.count().desc()).limit(limit)))
    statement = select(Memory).where(Memory.owner_id == owner, Memory.lifecycle_state.in_((mem_types.LIFECYCLE_ACTIVE, mem_types.LIFECYCLE_CANDIDATE, "established", "latent")))
    if ids:
        statement = statement.where(Memory.id.in_(ids))
    elif query_terms:
        return []
    else:
        statement = statement.order_by(Memory.salience.desc(), Memory.updated_at.desc()).limit(limit)
    return list(db.scalars(statement))


def _base_score(row, query_terms, query_vector, preferred_scopes):
    data = row.data or {}
    row_terms = terms(" ".join((data.get("content", ""), data.get("gist", ""), *data.get("entities", []), *data.get("concepts", []))))
    overlap = len(query_terms & row_terms) / max(1, len(query_terms))
    semantic = cosine(query_vector, data.get("feature_vector", []))
    scope_factor = 1.0 if not preferred_scopes or data.get("scope", "user") in preferred_scopes else 0.82
    activated = row.last_activated_at or row.updated_at or row.created_at
    if activated.tzinfo is None:
        activated = activated.replace(tzinfo=UTC)
    recency = 1 / (1 + max(0, (datetime.now(UTC) - activated).days) / 180)
    value = 0.31 * overlap + 0.19 * semantic + 0.16 * row.confidence + 0.14 * row.strength + 0.10 * row.salience + 0.06 * recency + 0.04 * scope_factor
    state_value = _state(row)
    if state_value in ("latent", mem_types.LIFECYCLE_CANDIDATE):
        value *= 0.55 if overlap < 0.5 else 0.8
    return value, {"lexical": overlap, "feature": semantic, "scope": scope_factor, "recency": recency}


def search(db, owner, query="", scopes=None, *, settings=None, debug=False, activate=False):
    auto = _auto_config(db, owner)
    overrides = {key: value for key, value in (settings or {}).items() if key in DEFAULTS}
    cfg = {**DEFAULTS, **auto, **overrides}
    cfg["seed_limit"] = max(4, min(64, int(cfg["seed_limit"])))
    cfg["result_limit"] = max(1, min(30, int(cfg["result_limit"])))
    cfg["max_depth"] = max(0, min(3, int(cfg["max_depth"])))
    cfg["max_candidates"] = max(8, min(256, int(cfg["max_candidates"])))
    started = time.monotonic()
    deadline = started + max(60, min(400, int(cfg["retrieval_budget_ms"]))) / 1000
    query_terms, query_vector = terms(query), feature_vector(terms(query))
    seed_rows = _candidate_rows(db, owner, query_terms, min(cfg["seed_limit"], cfg["max_candidates"]))
    scores, reasons, rows, frontier = {}, {}, {row.id: row for row in seed_rows}, {}
    steps, rejected = [], []
    for row in seed_rows:
        value, breakdown = _base_score(row, query_terms, query_vector, scopes or [])
        threshold = SEED_THRESHOLD.get(_state(row), SEED_THRESHOLD["established"])
        if value >= threshold:
            scores[row.id] = frontier[row.id] = value
            reasons[row.id] = breakdown
        else:
            rejected.append({"id": row.id, "score": round(value, 4), "threshold": threshold})
    steps.append({"stage": "seed", "found": len(seed_rows), "admitted": len(frontier), "rejected": rejected[:12], "thresholds": dict(SEED_THRESHOLD)})
    expanded = []
    stop_reason = "depth" if cfg["max_depth"] else "no_edges"
    for depth in range(cfg["max_depth"]):
        if not frontier:
            stop_reason = "no_frontier"
            break
        if len(scores) >= cfg["max_candidates"]:
            stop_reason = "candidate_budget"
            break
        if time.monotonic() >= deadline:
            stop_reason = "deadline"
            break
        frontier_ids = list(frontier)
        edge_statement = select(MemoryAssociation).where(MemoryAssociation.owner_id == owner, MemoryAssociation.source_memory_id.in_(frontier_ids))
        considered = int(db.scalar(select(func.count()).select_from(edge_statement.subquery())) or 0)
        associations = list(db.scalars(edge_statement.where(MemoryAssociation.weight >= float(cfg["min_association_weight"])).order_by(MemoryAssociation.weight.desc()).limit(cfg["max_candidates"])))
        target_ids = {item.target_memory_id for item in associations} - rows.keys()
        for row in db.scalars(select(Memory).where(Memory.owner_id == owner, Memory.id.in_(target_ids))) if target_ids else []:
            rows[row.id] = row
        next_frontier, accepted = {}, 0
        for association in associations:
            row = rows.get(association.target_memory_id)
            if not row or _state(row) not in ("established", "latent", mem_types.LIFECYCLE_ACTIVE, mem_types.LIFECYCLE_CANDIDATE):
                continue
            activation = frontier.get(association.source_memory_id, 0) * association.weight * float(cfg["activation_decay"])
            if activation <= scores.get(row.id, 0):
                continue
            base, breakdown = _base_score(row, query_terms, query_vector, scopes or [])
            scores[row.id], next_frontier[row.id] = base + activation, activation
            reasons[row.id] = {**breakdown, "association": activation, "depth": depth + 1}
            expanded.append({"source": association.source_memory_id, "target": row.id, "activation": round(activation, 4)})
            accepted += 1
            if len(scores) >= cfg["max_candidates"]:
                break
        steps.append({"stage": "hop", "depth": depth + 1, "edges_considered": considered, "edges_below_threshold": max(0, considered - len(associations)), "accepted": accepted, "frontier_after": len(next_frontier), "elapsed_ms": round((time.monotonic() - started) * 1000, 2)})
        frontier = next_frontier
    ranked = sorted(scores, key=lambda key: (scores[key], rows[key].updated_at), reverse=True)[:cfg["result_limit"]]
    if activate and ranked:
        activated_at = now()
        for key in ranked:
            rows[key].activation_count += 1
            rows[key].last_activated_at = rows[key].updated_at = activated_at
        reinforce_associations(db, owner, ranked)
    result = []
    for key in ranked:
        why = reasons[key]
        reason = "関連する記憶から連想" if why.get("association", 0) >= 0.1 else ("現在の話題と強く一致" if why.get("lexical", 0) >= 0.5 else ("現在の概念と関連" if why.get("feature", 0) >= 0.4 else "重要度と文脈から選択"))
        item = {"id": key, **_trace_data(rows[key]), "relevance": round(scores[key], 4), "selection_reason": reason}
        if debug:
            item["score_breakdown"] = {name: round(value, 4) if isinstance(value, float) else value for name, value in why.items()}
        result.append(item)
    steps.append({"stage": "rank", "considered": len(scores), "kept": len(ranked), "result_limit": cfg["result_limit"]})
    elapsed_ms = round((time.monotonic() - started) * 1000, 2)
    budget_exhausted = time.monotonic() >= deadline
    if stop_reason == "deadline":
        LOGGER.warning("memory retrieval budget exhausted owner=%s elapsed_ms=%s budget_ms=%s", owner, elapsed_ms, int(cfg["retrieval_budget_ms"]))
    if debug:
        return {"memories": result, "debug": {
            "auto": {key: cfg[key] for key in ("max_depth", "max_candidates", "retrieval_budget_ms", "min_association_weight", "activation_decay", "trace_count", "edge_count")},
            "trace": steps,
            "stop_reason": stop_reason,
            "elapsed_ms": elapsed_ms,
            "honored_settings": sorted(overrides),
            "ignored_settings": sorted(key for key in (settings or {}) if key in AUTO_OVERRIDES_IGNORED),
            "seed_ids": [row.id for row in seed_rows],
            "association_expansion": expanded,
            "budget_exhausted": budget_exhausted,
        }}
    return result


def reinforce_associations(db, owner, memory_ids):
    for source in memory_ids:
        for target in memory_ids:
            if source == target:
                continue
            row = db.scalar(select(MemoryAssociation).where(MemoryAssociation.owner_id == owner, MemoryAssociation.source_memory_id == source, MemoryAssociation.target_memory_id == target))
            if row:
                row.weight, row.confidence = clamp(row.weight + 0.02, 0, 0.95), clamp(row.confidence + 0.01)
                row.coactivation_count += 1
                row.updated_at = now()
            else:
                db.add(MemoryAssociation(owner_id=owner, source_memory_id=source, target_memory_id=target, weight=0.2, confidence=0.5, coactivation_count=1, data={"relation": "coactivated"}))


def _event(db, owner, run_id, action, memory_id, before, after, reason, candidate_index=0):
    db.add(MemoryActionEvent(owner_id=owner, run_id=run_id if run_id and run_id != "explicit-user-request" else None, candidate_index=candidate_index, data={"action": action, "memory_id": memory_id, "strength_before": before, "strength_after": after, "reason": reason}))


def change(db, owner, content=None, memory_id=None, delete=False, scope="user", source_run=None, scopes=None, *, importance=2, category=None, pinned=False, strength=None, confidence=None, salience=None, lifecycle_state=None, gist=None, entities=None, concepts=None, temporal_context=None, metadata=None, candidate_index=0, role=None, source_kind=None, role_metadata=None, verification=None):
    if content is not None and SENSITIVE.search(content):
        raise ValueError("Memoryには認証情報・トークン・パスワードを保存できません")
    content = re.sub(r"\s+", " ", content).strip() if content is not None else None
    # Role taxonomy columns live on the row as well as in the JSONB blob, so an
    # explicit edit keeps both copies in step with the runtime's writer.
    if role is not None and role not in mem_types.ROLES:
        raise ValueError(f"Unknown memory role: {role}")
    if source_kind is not None and source_kind not in mem_types.SOURCES:
        raise ValueError(f"Unknown memory source_kind: {source_kind}")
    if verification is not None and verification not in mem_types.VERIFICATIONS:
        raise ValueError(f"Unknown memory verification: {verification}")
    # Scopes are the five taxonomy buckets plus the per-agent "agent:<id>" form.
    if scope not in mem_types.SCOPES and not scope.startswith("agent:"):
        raise ValueError(f"Unknown memory scope: {scope}")
    explicit = source_run == "explicit-user-request" or pinned or importance >= 4
    target_salience = salience if salience is not None else max(0.2, min(1.0, importance / 5))
    target_strength = strength if strength is not None else (0.85 if explicit else 0.45)
    target_confidence = confidence if confidence is not None else (0.95 if explicit else 0.7)
    if memory_id:
        row = db.get(Memory, memory_id)
        if not row or row.owner_id != owner:
            raise ValueError("Memory not found")
        db.add(MemoryRevision(owner_id=owner, data={"memory_id": row.id, "previous": _trace_data(row)}))
        data = dict(row.data or {})
        for name, value in (("content", content), ("gist", gist), ("entities", entities), ("concepts", concepts), ("temporal_context", temporal_context)):
            if value is not None:
                data[name] = value
        if metadata is not None:
            data["metadata"] = {**data.get("metadata", {}), **metadata}
        if category:
            data["metadata"] = {**data.get("metadata", {}), "legacy_category": category}
        if role_metadata is not None:
            data["role_metadata"] = {**(data.get("role_metadata") or {}), **role_metadata}
        if role is not None:
            data["role"] = role
        if source_kind is not None:
            data["source_kind"] = source_kind
        if verification is not None:
            data["verification"] = verification
        data.update({"scope": scope or data.get("scope", "user"), "source_run": source_run or data.get("source_run"), "deleted": delete, "pinned": pinned})
        row.data = data
        if scope:
            row.scope = scope
        if role is not None:
            row.role = role
        if source_kind is not None:
            row.source_kind = source_kind
        if verification is not None:
            row.verification = verification
        lifecycle_value = lifecycle_state or row.lifecycle_state or mem_types.LIFECYCLE_ACTIVE
        if lifecycle_value == mem_types.LIFECYCLE_ACTIVE:
            lifecycle_value = "established"
        elif lifecycle_value == mem_types.LIFECYCLE_CANDIDATE:
            lifecycle_value = "latent"
        elif lifecycle_value == mem_types.LIFECYCLE_EXPIRED:
            lifecycle_value = "deleted"
        row.lifecycle_state = "deleted" if delete else lifecycle_value
        row.strength, row.confidence, row.salience, row.updated_at = clamp(target_strength), clamp(target_confidence), clamp(target_salience), now()
    else:
        if not content:
            raise ValueError("Memory content is required")
        normalized = content.casefold()
        existing = next((candidate for candidate in _candidate_rows(db, owner, terms(content), 24) if _state(candidate) not in ("deleted", mem_types.LIFECYCLE_EXPIRED) and candidate.data.get("content", "").casefold() == normalized), None)
        if existing:
            before = existing.strength
            existing.strength = clamp(existing.strength + (0.14 if explicit else 0.08))
            existing.confidence = clamp(max(existing.confidence, target_confidence) + 0.03)
            existing.salience, existing.last_reinforced_at, existing.updated_at = clamp(max(existing.salience, target_salience)), now(), now()
            if existing.lifecycle_state in ("latent", mem_types.LIFECYCLE_CANDIDATE) and existing.strength >= 0.65 and existing.confidence >= 0.72:
                existing.lifecycle_state = "established" if existing.lifecycle_state == "latent" else mem_types.LIFECYCLE_ACTIVE
            _event(db, owner, source_run, "REINFORCE_TRACE", existing.id, before, existing.strength, "equivalent trace", candidate_index)
            return {"id": existing.id, **_trace_data(existing), "deduplicated": True}
        state = lifecycle_state or ("established" if explicit or target_confidence >= 0.82 else "latent")
        if state == mem_types.LIFECYCLE_ACTIVE:
            state = "established"
        elif state == mem_types.LIFECYCLE_CANDIDATE:
            state = "latent"
        trace_metadata = dict(metadata or {})
        if category:
            trace_metadata["legacy_category"] = category
        row = Memory(owner_id=owner, lifecycle_state=state, role=role or mem_types.ROLE_FACT, scope=scope, source_kind=source_kind or mem_types.SOURCE_USER, verification=verification or mem_types.VERIFICATION_UNVERIFIED, strength=clamp(target_strength), confidence=clamp(target_confidence), salience=clamp(target_salience), last_reinforced_at=now(), updated_at=now(), data={"content": content, "gist": gist or content[:300], "scope": scope, "role": role or mem_types.ROLE_FACT, "source_kind": source_kind or mem_types.SOURCE_USER, "verification": verification or mem_types.VERIFICATION_UNVERIFIED, "role_metadata": dict(role_metadata or {}), "source_run": source_run, "deleted": False, "pinned": pinned, "entities": entities or [], "concepts": concepts or [], "temporal_context": temporal_context, "metadata": trace_metadata})
        db.add(row)
        db.flush()
    _sync_features(db, row)
    if delete:
        for association in db.scalars(select(MemoryAssociation).where(MemoryAssociation.owner_id == owner, MemoryAssociation.source_memory_id == row.id)):
            derived = db.get(Memory, association.target_memory_id)
            if derived and row.id in (derived.data.get("metadata", {}).get("source_refs") or []):
                derived.strength = clamp(derived.strength - 0.35)
                if derived.strength < 0.3:
                    derived.lifecycle_state = "archived"
                derived.updated_at = now()
    _event(db, owner, source_run, "DELETE_TRACE" if delete else ("UPDATE_TRACE" if memory_id else "CREATE_TRACE"), row.id, None, row.strength, "explicit action" if explicit else "validated candidate", candidate_index)
    db.flush()
    return {"id": row.id, **_trace_data(row)}


def list_traces(db, owner, query="", state=None, limit=200):
    statement = select(Memory).where(Memory.owner_id == owner)
    if state:
        statement = statement.where(Memory.lifecycle_state == state)
    rows = list(db.scalars(statement.order_by(Memory.updated_at.desc()).limit(min(limit, 500))))
    query_terms = terms(query)
    if query_terms:
        rows = [row for row in rows if query_terms & terms(row.data.get("content", ""))]
    return [{"id": row.id, **_trace_data(row)} for row in rows]


def associations_for(db, owner, memory_id, limit=20):
    rows = list(db.scalars(
        select(MemoryAssociation)
        .where(MemoryAssociation.owner_id == owner, or_(MemoryAssociation.source_memory_id == memory_id, MemoryAssociation.target_memory_id == memory_id))
        .order_by(MemoryAssociation.weight.desc())
        .limit(limit)
    ))
    connected_ids = {row.target_memory_id if row.source_memory_id == memory_id else row.source_memory_id for row in rows}
    connected = {row.id: row for row in db.scalars(select(Memory).where(Memory.owner_id == owner, Memory.id.in_(connected_ids)))}
    result = []
    for row in rows:
        peer_id = row.target_memory_id if row.source_memory_id == memory_id else row.source_memory_id
        peer = connected.get(peer_id)
        result.append({
            "id": row.id, "source_memory_id": row.source_memory_id, "target_memory_id": row.target_memory_id,
            "weight": row.weight, "confidence": row.confidence, "relation": row.data.get("relation"),
            "connected_memory": {"id": peer.id, "content": peer.data.get("content", ""), "lifecycle_state": peer.lifecycle_state} if peer else None,
        })
    return result


def restore(db, owner, row, previous):
    db.add(MemoryRevision(owner_id=owner, data={"memory_id": row.id, "previous": _trace_data(row)}))
    structural = {"lifecycle_state", "strength", "confidence", "salience", "activation_count", "last_activated_at", "last_reinforced_at"}
    row.data = {key: value for key, value in previous.items() if key not in structural and key not in ("importance", "relevance", "selection_reason", "score_breakdown")}
    row.lifecycle_state = previous.get("lifecycle_state", "established")
    row.strength = clamp(previous.get("strength", .6))
    row.confidence = clamp(previous.get("confidence", .7))
    row.salience = clamp(previous.get("salience", .5))
    row.activation_count = int(previous.get("activation_count", 0))
    row.last_activated_at = datetime.fromisoformat(previous["last_activated_at"]) if previous.get("last_activated_at") else None
    row.last_reinforced_at = datetime.fromisoformat(previous["last_reinforced_at"]) if previous.get("last_reinforced_at") else None
    row.updated_at = now()
    _sync_features(db, row)
    return {"id": row.id, **_trace_data(row)}


def decay(db, owner, limit=200):
    changed = 0
    for row in db.scalars(select(Memory).where(Memory.owner_id == owner, Memory.lifecycle_state.in_((mem_types.LIFECYCLE_CANDIDATE, mem_types.LIFECYCLE_ACTIVE, "latent", "established"))).order_by(Memory.updated_at).limit(limit)):
        updated = row.updated_at.replace(tzinfo=UTC) if row.updated_at.tzinfo is None else row.updated_at
        age_days = (datetime.now(UTC) - updated).days
        if age_days < 30 or row.data.get("pinned"):
            continue
        row.strength = clamp(row.strength - min(0.18, age_days / 3650) * (1.2 - row.salience))
        row.updated_at = now()
        if row.lifecycle_state in ("latent", mem_types.LIFECYCLE_CANDIDATE) and row.strength < 0.15:
            row.lifecycle_state = mem_types.LIFECYCLE_ARCHIVED
        changed += 1
    return changed
