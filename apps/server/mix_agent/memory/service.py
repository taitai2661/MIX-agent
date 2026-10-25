"""Memory service: flat rows that carry the spec §1 taxonomy as real columns.

Rows stay ChatGPT-flat (``content`` + ``data`` for the row-specific extras of
§1.1 — ``pinned`` / ``source`` / ``source_run`` / ``strength`` /
``activation_count`` / ``embedding``), and as of migration 0024 the eight §1
columns live on the row itself: ``lifecycle_state`` / ``superseded_by_id`` /
``disputed_by_id`` / ``confidence`` / ``salience`` / ``role`` / ``scope`` /
``source_kind``. Nothing echoes a constant back any more — ``_trace_data``
and ``views.render`` report what the row actually is.

Spec §4 is the other half of this module: ``memory_links`` holds one directed
edge per row, ``link()`` upserts §4's own edges idempotently (the two §3
mirror kinds are refused — they exist only as a derivation of the §3
columns), ``sync_links()`` derives those two from ``superseded_by_id`` /
``disputed_by_id`` in the single §3 → §4 direction, and a search follows at
most one hop out of its seeds (``_expand_links``).

API kept backward-compatible where cheap (``**kwargs`` ignored) so callers
like engine/routes/tools keep working.
"""

from __future__ import annotations

import re
from datetime import UTC, datetime

from sqlalchemy import and_, inspect, or_, select

from mix_agent.db.models import Memory, MemoryActionEvent, MemoryLink, MemoryRevision, now
from mix_agent.memory import embeddings as mem_embeddings
from mix_agent.memory.types import (
    LEGACY_LIFECYCLE_MAP,
    LIFECYCLE_ACTIVE,
    LIFECYCLE_ARCHIVED,
    LIFECYCLE_CANDIDATE,
    LIFECYCLE_ESTABLISHED,
    LIFECYCLE_EXPIRED,
    LIFECYCLE_RETRIEVABLE,
    LIFECYCLE_SUPERSEDED,
    LIFECYCLES,
    REL_CONTRADICTS,
    REL_RELATES_TO,
    REL_SUPERSEDES,
    RELATION_KINDS,
    ROLE_FACT,
    ROLES,
    SCOPE_USER,
    SCOPES,
    SOURCE_AGENT,
    SOURCES,
)

SENSITIVE = re.compile(
    r"(?:"
    r"api[_ -]?key|password|passwd|secret|access[_ -]?token|session[_ -]?token"
    r"|authorization|private[_ -]?key|cookie"
    r"|bearer|client[_ -]?secret|refresh[_ -]?token"
    r"|認証コード|パスワード"
    r"|kennwort|passwort|zugangstoken|geheimschlüssel"
    r"|密码|访问令牌|密钥|认证码"
    r"|비밀번호|액세스토큰|인증코드"
    r")\s*[:=：]",
    re.IGNORECASE,
)
SENSITIVE_VALUE = re.compile(
    r"(?:"
    r"api[_ -]?key|password|passwd|secret|access[_ -]?token|session[_ -]?token"
    r"|authorization|private[_ -]?key|cookie"
    r"|bearer|client[_ -]?secret|refresh[_ -]?token"
    r"|認証コード|パスワード"
    r"|kennwort|passwort|zugangstoken|geheimschlüssel"
    r"|密码|访问令牌|密钥|认证码"
    r"|비밀번호|액세스토큰|인증코드"
    r")\s*[:=：]\s*[^\s,;]+",
    re.IGNORECASE,
)
STOP_WORDS = frozenset(
    {"これ", "それ", "ため", "です", "ます", "する", "した", "して", "with", "that", "this", "from", "your"}
)

# --- memory spec §2 `ranking` -------------------------------------------------
# Checked against the Desktop edition's constants by scripts/check-memory-parity.py.
# Change both sides (and the spec) together or CI fails.
LEXICAL_WEIGHT_WITH_VECTOR = 0.55
VECTOR_WEIGHT_WITH_VECTOR = 0.30
RECENCY_WEIGHT_WITH_VECTOR = 0.15
LEXICAL_WEIGHT_WITHOUT_VECTOR = 0.85
RECENCY_WEIGHT_WITHOUT_VECTOR = 0.15
MIN_SCORE = 0.10
RESULT_LIMIT_DEFAULT = 8
RESULT_LIMIT_MAX = 30
ITEM_CHAR_LIMIT = 600
# §1: `scope` is one of the eight columns search reads, and a caller's
# preferred scopes are a **soft** signal — a row in another scope stays
# retrievable, it only ranks lower. The factor is the one v0.3.5 shipped;
# a hard filter would hide a memory the user still wants to find.
SCOPE_MATCH_FACTOR = 1.0
SCOPE_MISMATCH_FACTOR = 0.82
# -----------------------------------------------------------------------------

# Rows considered per search. Ranking (especially the vector term) is brute
# force by design: a trigram/lexical prefilter would drop exactly the rows a
# semantic query needs. Personal stores stay far below this ceiling; raise it
# only with a measurement to match.
CANDIDATE_LIMIT = 2000

# --- memory spec §1 defaults and §3 decay/reinforce ---------------------------
# Spec §3 publishes the two decay numbers below; §1 owns the defaults.
# (Ranking constants are checked against the Desktop edition by
# scripts/check-memory-parity.py — these are not part of that contract.)
DEFAULT_LIFECYCLE = LIFECYCLE_ESTABLISHED  # §1 default / §2.1 reading of flat rows
DEFAULT_CONFIDENCE = 0.7
DEFAULT_SALIENCE = 0.5
DEFAULT_STRENGTH = 0.6  # §1.1: strength is held in `data`, not in a column
DECAY_HALF_LIFE_DAYS = 180.0  # §3: half-life on the same scale §5.1 gives recency
DECAY_FLOOR = 0.1  # §3: at/below this a memory stops being retrievable -> archived
DECAY_MIN_DAYS = 0.5  # §3: a daily job has nothing to say about a memory from an hour ago
REINFORCE_SALIENCE_STEP = 0.05
REINFORCE_CONFIDENCE_STEP = 0.02
# -----------------------------------------------------------------------------

# --- memory spec §4 `memory_links` --------------------------------------------
# §4's three generation sources plus the two caps §4 leaves to the
# implementation. Nothing here is part of scripts/check-memory-parity.py's
# vocabulary contract (that one owns §2/§6), so change spec §4 and these
# together.
LINK_DEFAULT_WEIGHT = 0.2  # §4 `weight`: "既定 0.2"
LINK_DEGREE_LIMIT = 5  # §4: 許容度数 — links followed out of one seed
LINK_EXPANSION_LIMIT = 8  # §4: rows one search may add through links
LINK_SUGGEST_LIMIT = 3  # §4 source (1): neighbours a fresh memory may claim
LINK_NEIGHBOR_TOP_K = 3  # §4 source (3): embedding neighbours per memory
LINK_NEIGHBOR_MIN_SIMILARITY = 0.75  # §4 source (3): §5's "十分高い類似度"
# §4 source (3) is 既定オフ; jobs._apply reads the setting under this key.
LINK_NEIGHBOR_SETTING = "memory_link_embedding_neighbors"
# -----------------------------------------------------------------------------

DEFAULTS = {"result_limit": RESULT_LIMIT_DEFAULT}
# Everything the old taxonomy accepted is now ignored (kept only so old
# callers/tests don't crash on unexpected kwargs).
AUTOMATIC_SETTING_KEYS: tuple[str, ...] = ()
AUTO_OVERRIDES_IGNORED: tuple[str, ...] = ()


def clamp(value, low=0.0, high=1.0):
    return max(low, min(high, float(value)))


def number(value, default=0.0):
    """Read a §1 float out of storage without letting junk raise.

    Rows written before 0024 (or edited by hand) can carry anything in
    ``data``; a bad scalar must degrade to the spec default, not take a
    search or a migration down with it.
    """
    try:
        return clamp(value)
    except (TypeError, ValueError):
        return default


def terms(value):
    text = re.sub(r"\s+", " ", (value or "").casefold()).strip()
    latin = re.findall(r"[\w-]{2,}", text, flags=re.UNICODE)
    japanese = re.findall(r"[\u3040-\u30ff\u3400-\u9fff]{2,}", text)
    grams = [part[i : i + 2] for part in japanese for i in range(len(part) - 1)]
    return {part[:160] for part in [*latin, *grams] if part not in STOP_WORDS}


# Trailing punctuation belongs to the command, not to the memory: 「覚えて。」
# and "remember!" save the same thing. Covers the full/half-width pairs of
# 。.!?、, and the two colons used as command separators.
_EXPLICIT_STRIP = " \t\r\n。．.!！？?、,，:：;；…"

# A command whose payload is only an English connective ("remember that") names
# nothing to remember; treating it as a candidate would store junk.
_VAGUE_CANDIDATES = frozenset({"that", "this", "it", "those", "these", "so", "there"})

# Spec §6: 「覚えて：」「〜を忘れて」「remember …」 save/delete synchronously, so the
# recogniser has to survive 命令形・敬語・英語・句読点の揺れ. The first two
# patterns of each tuple are the original contract
# (tests/integration/test_core.py::test_memory_explicit_capture_only_accepts_clear_requests
# and tests/integration/test_unified_memory.py::test_secret_rejected_and_forget_parser)
# and keep matching exactly; everything after them is additive. Prefix forms
# require a separator (colon / comma / whitespace) so a sentence that merely
# *starts* with the verb — 「覚えてる？」 — is not read as a command, and suffix
# forms never accept a bare 「覚えて」 — 「これを覚えて」 stays a non-command.
_REMEMBER_PATTERNS = (
    r"^(?:覚えて|記憶して|remember)\s*[:：]\s*(.+)$",
    r"^(.{2,500}?)\s*(?:を)?(?:覚えておいて|記憶しておいて|remember this)\s*[。.!！]?$",
    (
        r"^(?:覚えておいて|覚えといて|覚えてください|覚えといてください|覚えておいてください|"
        r"記憶しておいて|記憶してください|記憶しておいてください|メモして|メモしておいて|"
        r"メモしてください|メモしておいてください|记住)\s*(?:[:：、,]|\s)\s*(.+)$"
    ),
    (
        r"^(.{2,500}?)\s*(?:を)?\s*(?:覚えておいてください|覚えといてください|覚えておいて|"
        r"覚えといて|覚えてください|記憶しておいてください|記憶しておいて|記憶してください|"
        r"メモしておいてください|メモしておいて|メモしてください|メモして)\s*[。.!！？?、,]*$"
    ),
    (
        r"^(?:please\s+)?(?:remember\s+that|remember\s+to|note\s+that|don'?t\s+forget|"
        r"do\s+not\s+forget|remember|note)\s*[:：]?\s*(.+)$"
    ),
    (
        r"^(.{2,500}?)\s*,\s*(?:please\s+)?(?:remember(?:\s+this)?|don'?t\s+forget|"
        r"do\s+not\s+forget)\s*[.!?,]*$"
    ),
)

# Same construction as _REMEMBER_PATTERNS: two original patterns, then 敬語・
# 英語・句読点. Only memory-specific verbs are accepted (忘れて / 記憶から消す /
# forget / remove from memory): 「ファイルを消して」 must not be read as "delete a
# memory", and bare English "delete X" is deliberately not a pattern at all.
_FORGET_PATTERNS = (
    r"^(?:忘れて|記憶から消して)\s*[:：]\s*(.+)$",
    r"^(.{2,500}?)\s*(?:を)?(?:忘れて|記憶から消して)\s*[。.!！]?$",
    (
        r"^(?:忘れてください|忘れておいて|忘れておいてください|記憶から消してください|"
        r"記憶から消しておいて)\s*(?:[:：、,]|\s)\s*(.+)$"
    ),
    (
        r"^(.{2,500}?)\s*(?:を)?\s*(?:忘れてください|忘れておいてください|忘れておいて|"
        r"記憶から消してください|記憶から消しておいて)\s*[。.!！？?、,]*$"
    ),
    (
        r"^(?:please\s+)?(?:forget\s+about|forget\s+that|forget|remove\s+from\s+memory|"
        r"erase\s+from\s+memory)\s*[:：]?\s*(.+)$"
    ),
)


def _explicit_match(patterns, content, *, block_sensitive):
    text = (content or "").strip()
    for pattern in patterns:
        match = re.match(pattern, text, re.IGNORECASE)
        if not match:
            continue
        candidate = match.group(1).strip(_EXPLICIT_STRIP)
        if not candidate or candidate.casefold() in _VAGUE_CANDIDATES:
            continue
        # Spec §8 applies to every path: a save command that carries a
        # credential stores nothing. Forgetting one is harmless, so only the
        # save side blocks on it.
        if block_sensitive and SENSITIVE.search(candidate):
            return None
        return candidate
    return None


def explicit_candidate(content):
    return _explicit_match(_REMEMBER_PATTERNS, content, block_sensitive=True)


def explicit_forget_candidate(content):
    return _explicit_match(_FORGET_PATTERNS, content, block_sensitive=False)


def redact_sensitive(content):
    value = re.sub(
        r"-----BEGIN [^-]+PRIVATE KEY-----.*?-----END [^-]+PRIVATE KEY-----",
        "[REDACTED SECRET]",
        content or "",
        flags=re.DOTALL | re.IGNORECASE,
    )
    return SENSITIVE_VALUE.sub("[REDACTED SECRET]", value)


def role_value(value):
    """Validated spec §2 ``role`` (lower-cased), or ``None`` when unusable.

    §1 only allows vocabulary roles on a row, but both the API (``MemoryInput``)
    and the extractor can hand us arbitrary text; an unknown string therefore
    falls back to the row default instead of being persisted as a new,
    unrenderable role.
    """
    role = str(value or "").strip().lower()
    return role if role in ROLES else None


def lifecycle_value(value):
    """Validated spec §2 ``lifecycle`` (§2.1 legacy vocabulary mapped), or ``None``.

    §2.1 is a *reading* rule: ``latent``/``deleted`` may be present on a row
    that predates the current vocabulary, but they are stored as
    ``candidate``/``expired`` from here on.
    """
    state = str(value or "").strip().lower()
    state = LEGACY_LIFECYCLE_MAP.get(state, state)
    return state if state in LIFECYCLES else None


def scope_value(value):
    """Validated spec §2 ``scope``, or ``None`` when unusable."""
    scope = str(value or "").strip().lower()
    return scope if scope in SCOPES else None


def scope_weight(row, preferred) -> float:
    """The soft scope signal for one row.

    A row whose §2 scope is one of the caller's preferred scopes keeps the
    full weight; a row in any other scope is damped instead of dropped, so
    a scope hint reorders the results without hiding anything.
    """
    if not preferred:
        return SCOPE_MATCH_FACTOR
    scope = scope_value(getattr(row, "scope", None)) or SCOPE_USER
    return SCOPE_MATCH_FACTOR if scope in preferred else SCOPE_MISMATCH_FACTOR


def source_kind_value(value):
    """Validated spec §2 ``source``, or ``None`` when unusable."""
    source = str(value or "").strip().lower()
    return source if source in SOURCES else None


def is_pinned(row) -> bool:
    """§1 ``pinned``: stored in ``data`` (§1.1) and exempt from decay."""
    data = getattr(row, "data", None)
    return bool(data.get("pinned")) if isinstance(data, dict) else False


# Sentinel: a caller of the pure ``rank()`` may hand us payloads that are not
# rows at all (the shared recall fixture ranks plain ids). Those carry no
# lifecycle and are always candidates.
_NO_LIFECYCLE = object()


def retrievable(row) -> bool:
    """Spec §3's retrieval gate.

    A row is a candidate when its lifecycle is one of §2's
    ``retrievable_lifecycles`` **or** it is pinned — §3 exempts pinned memory
    from every automatic retirement, so a pinned row stays findable whatever
    its lifecycle says. Objects with no lifecycle at all pass through.
    """
    state = getattr(row, "lifecycle_state", _NO_LIFECYCLE)
    if state is _NO_LIFECYCLE:
        return True
    if is_pinned(row):
        return True
    return (lifecycle_value(state) or DEFAULT_LIFECYCLE) in LIFECYCLE_RETRIEVABLE


def _content(row: Memory) -> str:
    if getattr(row, "content", None):
        return row.content
    return (row.data or {}).get("content", "") or (row.data or {}).get("gist", "")


def _trace_data(row: Memory) -> dict:
    """Flat public shape (ChatGPT-like).

    Every spec §1 key below is read back from the row: migration 0024
    restored the eight columns, so this function reports what the memory
    *is* instead of echoing constants (§1's own "don't lie to the trace"
    rule). ``strength`` and ``activation_count`` stay in ``data`` per §1.1.
    """
    data = dict(row.data or {})
    content = _content(row)
    return {
        "content": content,
        "gist": content[:300],
        "pinned": is_pinned(row),
        "source": data.get("source", "user"),
        "source_run": data.get("source_run"),
        "created_at": row.created_at.isoformat() if row.created_at else None,
        "updated_at": row.updated_at.isoformat() if getattr(row, "updated_at", None) else None,
        # spec §1, real values (validated against §2 on the way in).
        "lifecycle_state": lifecycle_value(getattr(row, "lifecycle_state", None)) or DEFAULT_LIFECYCLE,
        "superseded_by_id": getattr(row, "superseded_by_id", None),
        "disputed_by_id": getattr(row, "disputed_by_id", None),
        "role": role_value(getattr(row, "role", None)) or role_value(data.get("role")) or ROLE_FACT,
        "scope": scope_value(getattr(row, "scope", None)) or SCOPE_USER,
        "source_kind": source_kind_value(getattr(row, "source_kind", None)) or SOURCE_AGENT,
        "strength": number(data.get("strength"), DEFAULT_STRENGTH),
        "confidence": number(getattr(row, "confidence", None), DEFAULT_CONFIDENCE),
        "salience": number(getattr(row, "salience", None), DEFAULT_SALIENCE),
    }


def _event(db, owner, run_id, action, memory_id, before, after, reason, candidate_index=0):
    # `after` is the id the action moved *to* (RESOLVE's winner): §1's log has
    # to be able to reconstruct a two-row transition on its own.
    related = after if isinstance(after, str) and after else None
    db.add(
        MemoryActionEvent(
            owner_id=owner,
            run_id=run_id if run_id and run_id != "explicit-user-request" else None,
            candidate_index=candidate_index,
            data={"action": action, "memory_id": memory_id, "related_memory_id": related, "reason": reason},
        )
    )


def _normalize(text: str) -> str:
    return re.sub(r"\s+", " ", (text or "").strip().casefold())


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


def _score(
    query_terms: set[str], row_terms: set[str], updated_at, query_vec=None, stored_vec=None
) -> tuple[float, dict]:
    overlap = len(query_terms & row_terms) / max(1, len(query_terms)) if query_terms else 0.0
    try:
        ts = updated_at
        if ts is not None and ts.tzinfo is None:
            ts = ts.replace(tzinfo=UTC)
        age_days = max(0.0, (datetime.now(UTC) - ts).total_seconds() / 86400) if ts else 365.0
    except Exception:  # noqa: BLE001 - an unreadable timestamp falls back to the default age
        age_days = 365.0
    recency = 1 / (1 + age_days / 180)
    vector = mem_embeddings.cosine01(query_vec, stored_vec)
    # Spec §5.2 with §2.2: weights are chosen per (query, candidate) pair, so a
    # row that has no embedding yet is scored exactly as it was before the
    # vector term existed — introducing embedding must not demote old rows.
    if vector is None:
        value = LEXICAL_WEIGHT_WITHOUT_VECTOR * overlap + RECENCY_WEIGHT_WITHOUT_VECTOR * recency
        return value, {
            "lexical": round(overlap, 4),
            "recency": round(recency, 4),
            "weights": "without_vector",
        }
    value = (
        LEXICAL_WEIGHT_WITH_VECTOR * overlap
        + VECTOR_WEIGHT_WITH_VECTOR * vector
        + RECENCY_WEIGHT_WITH_VECTOR * recency
    )
    return value, {
        "lexical": round(overlap, 4),
        "vector": round(vector, 4),
        "recency": round(recency, 4),
        "weights": "with_vector",
    }


def rank(
    query_terms: set[str],
    candidates,
    *,
    query_vector=None,
    limit: int = RESULT_LIMIT_DEFAULT,
    scope_factor=None,
) -> list[dict]:
    """Score, threshold, sort and cap candidates. Pure: no database, no I/O.

    ``candidates`` yields ``(row_terms, updated_at, stored_vector, payload)``.
    This is the whole of spec §5.1/§5.2 extracted from :func:`search` so the
    ranking contract in ``tests/fixtures/memory_recall_cases.json`` can be
    asserted directly rather than re-derived inside each test.

    ``scope_factor`` is an optional ``payload -> float`` damping function:
    the caller's preferred scopes are a soft signal (§1), applied to the
    score before the threshold. It is absent from the shared fixture, which
    pins the ranking formula itself.
    """
    if not query_terms:
        return []
    scored = []
    for row_terms, updated_at, stored_vec, payload in candidates:
        # §3's retrieval gate also holds here: `search` pre-filters, but rank()
        # is public, so a superseded/archived row handed in directly is still
        # not allowed to win a slot (unless it is pinned — see retrievable()).
        if not retrievable(payload):
            continue
        value, breakdown = _score(query_terms, row_terms, updated_at, query_vector, stored_vec)
        if scope_factor is not None:
            value = value * scope_factor(payload)
        # An exact lexical hit survives even a low combined score, so a genuine
        # word match is never discarded for being old.
        if value >= MIN_SCORE or query_terms & row_terms:
            scored.append((value, updated_at, payload, breakdown))
    # A wholly unrelated query returns nothing rather than the least unrelated
    # rows (avoid polluting context).
    if not scored or max(item[0] for item in scored) < MIN_SCORE:
        scored = []
    scored.sort(key=lambda item: (item[0], item[1]), reverse=True)
    return [
        {"payload": item[2], "relevance": round(item[0], 4), "breakdown": item[3]} for item in scored[:limit]
    ]


def search(
    db,
    owner,
    query="",
    scopes=None,
    *,
    settings=None,
    debug=False,
    activate=False,
    query_vector=None,
    **kwargs,
):
    limit = RESULT_LIMIT_DEFAULT
    try:
        limit = max(1, min(RESULT_LIMIT_MAX, int((settings or {}).get("result_limit", RESULT_LIMIT_DEFAULT))))
    except Exception:  # noqa: BLE001, S110 - a malformed setting falls back to the default limit
        pass
    query = (query or "").strip()
    query_terms = terms(query)
    rows = list(
        db.scalars(
            select(Memory)
            .where(Memory.owner_id == owner)
            .order_by(Memory.updated_at.desc())
            .limit(CANDIDATE_LIMIT)
        )
    )
    # Spec §3: retrieval only sees retrievable lifecycles (§2), plus every
    # pinned row whatever its lifecycle. The listing endpoints keep showing
    # the rest — a superseded memory is hidden from recall, not from review.
    candidates = [row for row in rows if retrievable(row)]
    expansion = {"added": [], "edges": [], "cutoff": None}
    seeds: list[dict] = []
    # §1: the caller's preferred scopes are a soft signal, not a filter.
    preferred = [str(scope).strip().lower() for scope in (scopes or []) if str(scope or "").strip()]
    if not query_terms:
        # No query: the order is recency, with the scope hint first so a
        # caller that asked for a scope sees it at the top.
        ordered = sorted(
            candidates,
            key=lambda row: (
                scope_weight(row, preferred),
                getattr(row, "updated_at", None) or row.created_at,
            ),
            reverse=True,
        )
        seeds = [
            {"id": r.id, **_trace_data(r), "relevance": 0.0, "selection_reason": "recent"}
            for r in ordered[:limit]
        ]
        result = seeds
        mode = "recent"
    else:
        ranked = rank(
            query_terms,
            (
                (
                    terms(_content(row)),
                    getattr(row, "updated_at", None) or row.created_at,
                    mem_embeddings.row_vector(row.data),
                    row,
                )
                for row in candidates
            ),
            query_vector=query_vector,
            limit=limit,
            scope_factor=(lambda row: scope_weight(row, preferred)) if preferred else None,
        )
        hits = [item["payload"] for item in ranked]
        if activate:
            # §1/§3 reactivation: a hit is exactly when a memory should be
            # reinforced. Off by default — recall feeds the context on every
            # turn, and silently rewriting recency there would reorder the
            # next search; the explicit caller asks for it.
            for row in hits:
                reinforce_row(row)
                _event(db, owner, None, "REINFORCE", row.id, None, None, "search hit")
            # §4 source (2): the same explicit search is the only place links
            # are co-created from "these came back together". Context
            # injection searches without `activate`, so it cannot.
            _coactivate(db, owner, hits)
        seeds = [
            {
                "id": item["payload"].id,
                **_trace_data(item["payload"]),
                "relevance": item["relevance"],
                "selection_reason": "語彙が一致" if item["breakdown"]["lexical"] >= 0.4 else "文脈から関連",
            }
            for item in ranked
        ]
        # §4: one hop past the seeds, degree- and count-capped. Linked rows
        # are appended *after* the hits, never mixed into their order.
        expansion = _expand_links(db, owner, seeds)
        result = [*seeds, *expansion["added"]]
        mode = "hybrid" if query_vector else "lexical"
    if debug:
        with_vector = LEXICAL_WEIGHT_WITH_VECTOR if query_vector else LEXICAL_WEIGHT_WITHOUT_VECTOR
        return {
            "memories": result,
            "debug": {
                # `stop_reason` stays a string TraceView renders verbatim:
                # "flat" while nothing was pulled in through a link, "depth"
                # once §4's one hop actually added rows. `ranking_mode` and
                # the `association_*` keys below are the additive signal.
                "considered": len(candidates),
                "kept": len(result),
                "stop_reason": "depth" if expansion["edges"] else "flat",
                "ranking_mode": mode,
                "lifecycle_excluded": len(rows) - len(candidates),
                "elapsed_ms": 0.0,
                "auto": {},
                "trace": [],
                "seed_ids": [r["id"] for r in seeds],
                # §5.3 debug key, now carrying the rows §4 added: each entry is
                # {source, target, activation} (+ `kind`), source = a seed.
                "association_expansion": expansion["edges"],
                "association_expansion_count": len(expansion["edges"]),
                "association_cutoff": expansion["cutoff"],
                "budget_exhausted": False,
                "honored_settings": [],
                "ignored_settings": [],
                "query_vector": bool(query_vector),
                "weights": {
                    "lexical": with_vector,
                    "vector": VECTOR_WEIGHT_WITH_VECTOR if query_vector else 0.0,
                    "recency": RECENCY_WEIGHT_WITH_VECTOR if query_vector else RECENCY_WEIGHT_WITHOUT_VECTOR,
                },
                "lexical": with_vector,
                "min_score": MIN_SCORE,
            },
        }
    return result


def list_traces(db, owner, query="", state=None, limit=200):
    """The flat listing, optionally narrowed to one §2 ``lifecycle_state``.

    ``state`` used to be **accepted and ignored**: a filter that does nothing
    is worse than no filter at all, because the caller cannot tell "nothing
    matched" from "I filtered wrong" — the list simply looked empty. It now
    filters exactly, and a value outside §2's ``LIFECYCLES`` is refused rather
    than silently returning an empty page.

    This is a *listing* filter, not §2's retrieval gate: someone asking for
    ``archived`` means the archived rows, so ``pinned`` gets no exemption here
    (§3 exempts pinned rows from being hidden by **retrieval**, not from being
    named by a filter).
    """
    if state is not None and state not in LIFECYCLES:
        raise ValueError(f"unknown lifecycle_state: {state!r}")
    rows = list(
        db.scalars(
            select(Memory)
            .where(Memory.owner_id == owner)
            .order_by(Memory.updated_at.desc())
            .limit(min(limit, 500))
        )
    )
    qterms = terms(query)
    if qterms:
        rows = [r for r in rows if qterms & terms(_content(r))]
    if state is not None:
        rows = [r for r in rows if (r.lifecycle_state or "") == state]
    return [{"id": r.id, **_trace_data(r)} for r in rows]


# --- memory spec §4 `memory_links` ---------------------------------------------
# One directed edge per row. §4's rule for the two §3 kinds is absolute:
# `kind=supersedes` *is* `superseded_by_id` and `kind=contradicts` *is*
# `disputed_by_id` — the link is the mirror of the column, never a second
# owner of that state. The derivation runs one way only, §3 → §4: a §3
# transition writes the column and `sync_links()` turns it into edges, so
# nothing in this file can set a §3 column. `link()` therefore refuses both
# mirror kinds (§2 vocabulary or not) and makes only §4's own edges.


def _links_available(db) -> bool:
    """Whether §4's table exists *yet*, answered without swallowing errors.

    Two callers need "not yet": the Phase 3 scratch-database test migrates a
    throw-away database only as far as 0024, and a deployment is briefly
    between revisions while ``alembic upgrade`` runs. Both must degrade to
    "no links" instead of raising on a missing table. The answer comes from
    the catalog and is cached on the session (it cannot change underneath a
    live session) — never from catching ``ProgrammingError``, which would
    also hide a genuine failure of the query itself.
    """
    cached = db.info.get("memory_links_available")
    if cached is None:
        cached = inspect(db.get_bind()).has_table("memory_links")
        db.info["memory_links_available"] = bool(cached)
    return bool(cached)


def _link_view(row: MemoryLink) -> dict:
    """§4's public edge shape, floats and ids only (no ORM objects leak)."""
    return {
        "id": row.id,
        "owner_id": row.owner_id,
        "source_memory_id": row.source_memory_id,
        "target_memory_id": row.target_memory_id,
        "kind": row.kind,
        "weight": number(row.weight, LINK_DEFAULT_WEIGHT),
        "coactivation_count": int(row.coactivation_count or 0),
        "created_at": row.created_at.isoformat() if row.created_at else None,
        "updated_at": row.updated_at.isoformat() if row.updated_at else None,
    }


def _upsert_link(db, owner, source_id, target_id, kind, *, weight=LINK_DEFAULT_WEIGHT, increment=False):
    """One row per ``(owner, source, target, kind)`` — §4's uniqueness rule.

    Returns ``(row, created)``. ``weight`` is written on creation and, for an
    existing edge, only when the caller states something other than the §4
    default: a coactivation increment must never drag a similarity-weighted
    edge back down to 0.2.
    """
    row = db.scalar(
        select(MemoryLink).where(
            MemoryLink.owner_id == owner,
            MemoryLink.source_memory_id == source_id,
            MemoryLink.target_memory_id == target_id,
            MemoryLink.kind == kind,
        )
    )
    created = row is None
    if created:
        row = MemoryLink(
            owner_id=owner,
            source_memory_id=source_id,
            target_memory_id=target_id,
            kind=kind,
            weight=clamp(weight),
            coactivation_count=1 if increment else 0,
            created_at=now(),
            updated_at=now(),
        )
        db.add(row)
    else:
        if increment:
            row.coactivation_count = int(row.coactivation_count or 0) + 1
        if weight != LINK_DEFAULT_WEIGHT:
            row.weight = clamp(weight)
        row.updated_at = now()
    db.flush()
    return row, created


def link(db, owner, source_id, target_id, kind, *, weight=LINK_DEFAULT_WEIGHT, increment=False) -> dict:
    """Spec §4: idempotent upsert of one directed edge.

    ``kind`` must be one of §2's ``relation_kinds`` — anything else raises
    ``ValueError`` first, so an unknown word never reaches the table.

    The two §3 mirror kinds are then refused **even though §2 defines them**:
    §4 writes only §4's own edges (``relates_to`` / ``supports`` /
    ``derived_from`` / ``caused_by``). ``superseded_by_id`` / ``disputed_by_id``
    are §3 columns, only a §3 transition may set them, and :func:`sync_links`
    turns them into edges — the derivation runs §3 → §4 and never back. Letting
    this function write a column instead would raise a row that carries
    ``superseded_by_id`` while its ``lifecycle_state`` stays ``established``,
    i.e. one still in §2's ``retrievable_lifecycles`` although §3's diagram has
    ``supersede`` / ``dispute`` set the state *and* the pointer together.
    """
    if kind not in RELATION_KINDS:
        raise ValueError(f"unknown relation kind: {kind!r}")
    if kind in (REL_SUPERSEDES, REL_CONTRADICTS):
        raise ValueError(
            f"{kind!r} is the mirror of §3's superseded_by_id / disputed_by_id: only a §3"
            " transition writes those columns and sync_links derives the edge from them"
            " (spec §4); link() makes §4's own edges only"
        )
    source = db.get(Memory, str(source_id)) if source_id else None
    target = db.get(Memory, str(target_id)) if target_id else None
    if source is None or source.owner_id != owner:
        raise ValueError("source memory not found")
    if target is None or target.owner_id != owner:
        raise ValueError("target memory not found")
    if source.id == target.id:
        raise ValueError("a memory cannot link to itself")
    row, _created = _upsert_link(db, owner, source.id, target.id, kind, weight=weight, increment=increment)
    return _link_view(row)


def sync_links(db, row) -> None:
    """§4's mirror: make the edges around ``row`` agree with its §3 columns.

    Called by every transition that moves ``superseded_by_id`` /
    ``disputed_by_id`` — supersede, dispute, resolve, revision restore,
    delete — inside the same transaction, so no path can leave a column
    naming an edge that is not there or an edge claiming a pointer the row no
    longer holds. Inactive rows (missing table, pre-0025 database) are a
    no-op, not an error.
    """
    if row is None or not _links_available(db):
        return
    _sync_kind(db, row, REL_SUPERSEDES, getattr(row, "superseded_by_id", None))
    _sync_kind(db, row, REL_CONTRADICTS, getattr(row, "disputed_by_id", None))


def _sync_kind(db, row, kind, source_id):
    """Reconcile one §4 edge set with one §3 column (see :func:`sync_links`)."""
    existing = list(
        db.scalars(
            select(MemoryLink).where(
                MemoryLink.owner_id == row.owner_id,
                MemoryLink.target_memory_id == row.id,
                MemoryLink.kind == kind,
            )
        )
    )
    keeper = None
    for edge in existing:
        if keeper is None and edge.source_memory_id == source_id:
            keeper = edge  # the edge the column names: keep it as it is
            continue
        db.delete(edge)  # a stale mirror (or an edge whose pointer moved)
    if source_id and keeper is None:
        pointed = db.get(Memory, str(source_id))
        # A pointer whose row is gone gets no edge — the foreign key refuses
        # one — and `sync_links` leaves the column for its caller to judge.
        if pointed is not None and pointed.owner_id == row.owner_id:
            db.add(
                MemoryLink(
                    owner_id=row.owner_id,
                    source_memory_id=pointed.id,
                    target_memory_id=row.id,
                    kind=kind,
                    weight=LINK_DEFAULT_WEIGHT,
                    coactivation_count=0,
                    created_at=now(),
                    updated_at=now(),
                )
            )
    db.flush()


def associations_for(db, owner, memory_id, limit=20):
    """Spec §4: the edges touching ``memory_id``, in both directions.

    Returned newest-heaviest first and capped by ``limit``; §4's endpoint is
    an inspection view, not a retrieval path, so the lifecycle gate does not
    apply here — a superseded row's edges are exactly what a reviewer wants
    to see.
    """
    if not memory_id or not _links_available(db):
        return []
    rows = list(
        db.scalars(
            select(MemoryLink)
            .where(
                MemoryLink.owner_id == owner,
                or_(MemoryLink.source_memory_id == memory_id, MemoryLink.target_memory_id == memory_id),
            )
            .order_by(
                MemoryLink.weight.desc(), MemoryLink.coactivation_count.desc(), MemoryLink.updated_at.desc()
            )
            .limit(max(1, int(limit or 20)))
        )
    )
    return [_link_view(row) for row in rows]


def link_counts(db, owner, ids) -> dict[str, int]:
    """How many edges touch each id, in one round trip (for §5.3 rendering)."""
    wanted = [str(item) for item in (ids or []) if item]
    if not wanted or not _links_available(db):
        return {}
    counts: dict[str, int] = {}
    for source_id, target_id in db.execute(
        select(MemoryLink.source_memory_id, MemoryLink.target_memory_id).where(
            MemoryLink.owner_id == owner,
            or_(MemoryLink.source_memory_id.in_(wanted), MemoryLink.target_memory_id.in_(wanted)),
        )
    ):
        counts[source_id] = counts.get(source_id, 0) + 1
        counts[target_id] = counts.get(target_id, 0) + 1
    return counts


def _coactivate(db, owner, rows) -> int:
    """§4 source (2): rows recalled together by one *explicit* search.

    Pairwise over the hit list (rank order fixes the direction), ``kind
    relates_to``, +1 per repeat. Gated on ``activate=True`` so context
    injection — which searches every turn without activating — cannot fabric
    links out of "these two were mentioned today"; that is the same reason
    §3 keeps injection from reinforcing.
    """
    ids = [row.id for row in rows]
    if len(ids) < 2:
        return 0
    created = 0
    for index, source_id in enumerate(ids):
        for target_id in ids[index + 1 :]:
            # A coactivation is symmetric, so the pair is *one* row however the
            # two came back this time: §5.2 breaks a tie on `updated_at`, which
            # the reinforcing above just moved, so the order can flip between
            # searches. An edge already stored the other way round is the same
            # fact — increment it instead of adding its mirror image.
            existing = db.scalar(
                select(MemoryLink).where(
                    MemoryLink.owner_id == owner,
                    MemoryLink.kind == REL_RELATES_TO,
                    or_(
                        and_(
                            MemoryLink.source_memory_id == source_id, MemoryLink.target_memory_id == target_id
                        ),
                        and_(
                            MemoryLink.source_memory_id == target_id, MemoryLink.target_memory_id == source_id
                        ),
                    ),
                )
            )
            if existing is not None:
                existing.coactivation_count = int(existing.coactivation_count or 0) + 1
                existing.updated_at = now()
                db.flush()
                continue
            _row, is_new = _upsert_link(db, owner, source_id, target_id, REL_RELATES_TO, increment=True)
            created += 1 if is_new else 0
    return created


def _expand_links(
    db, owner, seeds, *, degree_limit=LINK_DEGREE_LIMIT, total_limit=LINK_EXPANSION_LIMIT
) -> dict:
    """§4: exactly one hop out of the seed rows — never two.

    The seeds are the rows the query actually matched; a linked row is
    appended *after* them with ``selection_reason="リンクから"`` and a
    relevance the seed's relevance was scaled down by the edge weight, so it
    can neither outrank a real hit nor be mistaken for one. Both §4 caps are
    honoured here: ``degree_limit`` edges per seed, ``total_limit`` rows per
    search, whichever comes first (``cutoff`` says which one did).

    ``retrievable()`` is applied again on the way in: a link into a
    superseded or archived memory does not resurrect it, while a pinned one
    stays reachable (§3 exempts pinned rows from every automatic retirement).
    """
    empty = {"added": [], "edges": [], "cutoff": None}
    if not seeds or not _links_available(db):
        return empty
    seed_ids = [seed["id"] for seed in seeds]
    seed_set = set(seed_ids)
    relevance_of = {seed["id"]: number(seed.get("relevance"), 0.0) for seed in seeds}
    links = list(
        db.scalars(
            select(MemoryLink)
            .where(
                MemoryLink.owner_id == owner,
                or_(MemoryLink.source_memory_id.in_(seed_ids), MemoryLink.target_memory_id.in_(seed_ids)),
            )
            .order_by(
                MemoryLink.weight.desc(), MemoryLink.coactivation_count.desc(), MemoryLink.updated_at.desc()
            )
        )
    )
    reached: dict[str, tuple[MemoryLink, str]] = {}
    degree: dict[str, int] = {}
    degree_truncated = False
    for edge in links:
        seed_id = edge.source_memory_id if edge.source_memory_id in seed_set else edge.target_memory_id
        other_id = edge.target_memory_id if seed_id == edge.source_memory_id else edge.source_memory_id
        if other_id in seed_set:
            continue  # seed ↔ seed: both rows are already in the result
        if degree.get(seed_id, 0) >= degree_limit:
            degree_truncated = True  # §4's degree cap stops this seed here
            continue
        degree[seed_id] = degree.get(seed_id, 0) + 1
        if other_id not in reached:
            reached[other_id] = (edge, seed_id)
    added: list[dict] = []
    edges: list[dict] = []
    cutoff = None
    for other_id, (edge, seed_id) in reached.items():
        if len(added) >= total_limit:
            cutoff = "expansion_limit"
            break
        row = db.get(Memory, other_id)
        if row is None or row.owner_id != owner or not retrievable(row):
            continue
        weight = number(edge.weight, LINK_DEFAULT_WEIGHT)
        added.append(
            {
                "id": row.id,
                **_trace_data(row),
                "relevance": round(relevance_of.get(seed_id, 0.0) * weight, 4),
                "selection_reason": "リンクから",
            }
        )
        edges.append({"source": seed_id, "target": row.id, "activation": weight, "kind": edge.kind})
    if cutoff is None and degree_truncated:
        cutoff = "degree_limit"
    return {"added": added, "edges": edges, "cutoff": cutoff}


def suggest_links(db, owner, memory_ids, *, limit=LINK_SUGGEST_LIMIT) -> int:
    """§4 source (1): the links an extraction *would* have proposed.

    §6's prompt is a verbatim cross-edition contract — parity byte-compares
    it against the Desktop copy and ``test_memory_extraction_quality``
    greps the spec for it — so asking the model for a ``links`` key would
    mean changing §6 and the other edition in lockstep, not adding a field.
    The proposal is therefore derived after ``jobs._apply`` from what §5
    already agrees on: the new row's terms against the owner's store, ranked
    by the same :func:`rank` §4's retrieval uses, capped at ``limit`` per
    memory and restricted to rows §3 still lets retrieval see.
    """
    ids = [str(item) for item in (memory_ids or []) if item]
    if not ids or not _links_available(db):
        return 0
    pool = list(
        db.scalars(
            select(Memory).where(Memory.owner_id == owner).order_by(Memory.updated_at.desc()).limit(200)
        )
    )
    created = 0
    for memory_id in ids:
        row = db.get(Memory, memory_id)
        if row is None or row.owner_id != owner:
            continue
        query_terms = terms(_content(row))
        if not query_terms:
            continue
        matches = [other for other in pool if other.id != row.id and query_terms & terms(_content(other))]
        for item in rank(
            query_terms,
            (
                (
                    terms(_content(other)),
                    other.updated_at or other.created_at,
                    mem_embeddings.row_vector(other.data),
                    other,
                )
                for other in matches
            ),
            limit=limit,
        ):
            _edge, is_new = _upsert_link(db, owner, row.id, item["payload"].id, REL_RELATES_TO)
            created += 1 if is_new else 0
    return created


def embedding_neighbor_links(
    db, owner, memory_ids, *, top_k=LINK_NEIGHBOR_TOP_K, minimum=LINK_NEIGHBOR_MIN_SIMILARITY
) -> int:
    """§4 source (3): §5 embedding neighbours — **既定オフ**.

    :func:`jobs._apply` only calls this when the ``memory_link_embedding_
    neighbors`` setting is true: links made here grow with the square of the
    store, and §4 marks the source off by default for exactly that reason.
    Rows without a vector, and rows §3 would not retrieve, never become
    neighbours.
    """
    ids = [str(item) for item in (memory_ids or []) if item]
    if not ids or not _links_available(db):
        return 0
    pool = [
        row
        for row in db.scalars(
            select(Memory)
            .where(Memory.owner_id == owner)
            .order_by(Memory.updated_at.desc())
            .limit(CANDIDATE_LIMIT)
        )
        if row.id not in ids and retrievable(row)
    ]
    created = 0
    for memory_id in ids:
        row = db.get(Memory, memory_id)
        if row is None or row.owner_id != owner:
            continue
        vector = mem_embeddings.row_vector(row.data)
        if vector is None:
            continue
        scored = []
        for other in pool:
            similarity = mem_embeddings.cosine01(vector, mem_embeddings.row_vector(other.data))
            if similarity is None or similarity < minimum:
                continue
            scored.append((similarity, other))
        scored.sort(key=lambda item: item[0], reverse=True)
        for similarity, other in scored[:top_k]:
            _edge, is_new = _upsert_link(
                db,
                owner,
                row.id,
                other.id,
                REL_RELATES_TO,
                weight=max(LINK_DEFAULT_WEIGHT, round(similarity, 2)),
            )
            created += 1 if is_new else 0
    return created


def _detach_pointers(db, owner, memory_id) -> None:
    """Clear every §3 pointer that named a row that is being deleted.

    §4 forbids the half-updated state: the edges go with the row (the foreign
    key cascades), so a column left pointing at a memory that no longer
    exists must go too — otherwise the mirror would claim a link for a row
    that is not there.
    """
    for name, column in (
        ("superseded_by_id", Memory.superseded_by_id),
        ("disputed_by_id", Memory.disputed_by_id),
    ):
        for row in list(db.scalars(select(Memory).where(Memory.owner_id == owner, column == memory_id))):
            setattr(row, name, None)
            db.flush()
            sync_links(db, row)


# -----------------------------------------------------------------------------


def restore(db, owner, row, previous):
    previous = previous if isinstance(previous, dict) else {}
    prev_content = (previous or {}).get("content", "") or (previous or {}).get("gist", "")
    if prev_content:
        row.content = str(prev_content)[:10000]
    data = dict(row.data or {})
    data["content"] = row.content
    if isinstance(previous, dict) and isinstance(previous.get("pinned"), bool):
        data["pinned"] = previous["pinned"]
    if isinstance(previous.get("source"), str):
        data["source"] = previous["source"]
    source_run = previous.get("source_run")
    if "source_run" in previous and (source_run is None or isinstance(source_run, str)):
        data["source_run"] = source_run
    if "strength" in previous:
        data["strength"] = number(previous.get("strength"), DEFAULT_STRENGTH)
    row.data = data
    # §1: restore the full taxonomy with the content, so no old lifecycle or
    # role is left paired with a different snapshot.
    for column, validator in (
        ("lifecycle_state", lifecycle_value),
        ("role", role_value),
        ("scope", scope_value),
        ("source_kind", source_kind_value),
    ):
        if column in previous:
            value = validator(previous.get(column))
            if value:
                setattr(row, column, value)
    for column, default in (("confidence", DEFAULT_CONFIDENCE), ("salience", DEFAULT_SALIENCE)):
        if column in previous:
            setattr(row, column, number(previous.get(column), default))
    # A deleted counterpart may no longer exist. Do not recreate a pointer or
    # mirror edge to a missing/cross-owner row from an old revision snapshot.
    for column in ("superseded_by_id", "disputed_by_id"):
        if column in previous:
            value = previous.get(column)
            pointed = db.get(Memory, value) if isinstance(value, str) and value else None
            if pointed is None or pointed.owner_id != owner:
                value = None
            setattr(row, column, value)
    row.updated_at = now()
    db.flush()
    # §4: the link mirror follows the columns it mirrors, in this transaction.
    sync_links(db, row)
    return {"id": row.id, **_trace_data(row)}


def restore_deleted(db, owner, memory_id, previous):
    """Recreate a forgotten Memory from its owner-scoped revision snapshot."""
    if not isinstance(previous, dict):
        raise TypeError("Memory revision snapshot is invalid")
    content = previous.get("content") or previous.get("gist")
    if not isinstance(content, str) or not content.strip():
        raise ValueError("Memory revision has no restorable content")
    content = content[:10000]
    row = Memory(
        id=memory_id,
        owner_id=owner,
        content=content,
        updated_at=now(),
        data={"content": content},
    )
    created_at = previous.get("created_at")
    if isinstance(created_at, str):
        try:
            restored_created_at = datetime.fromisoformat(created_at)
        except ValueError:
            pass
        else:
            row.created_at = (
                restored_created_at if restored_created_at.tzinfo else restored_created_at.replace(tzinfo=UTC)
            )
    db.add(row)
    db.flush()
    restored = restore(db, owner, row, previous)
    return row, restored


def reinforce_row(row, *, reason: str = "reinforce") -> Memory:
    """Spec §1/§3: one activation of a memory.

    ``updated_at`` moves (§1 — recency is measured from it), ``salience`` and
    ``confidence`` rise, ``data.strength`` rises with them, and the decay
    anchor is reset so a memory that was just recalled is not immediately
    decayed again. Bounded by ``clamp``: reinforcement approaches certainty,
    it never exceeds it.
    """
    row.updated_at = now()
    row.salience = clamp(number(row.salience, DEFAULT_SALIENCE) + REINFORCE_SALIENCE_STEP)
    row.confidence = clamp(number(row.confidence, DEFAULT_CONFIDENCE) + REINFORCE_CONFIDENCE_STEP)
    data = dict(row.data or {})
    data["strength"] = clamp(number(data.get("strength"), DEFAULT_STRENGTH) + REINFORCE_SALIENCE_STEP)
    data["activation_count"] = int(number(data.get("activation_count"), 0)) + 1
    data["last_decay_at"] = now().isoformat()
    row.data = data
    return row


def resolve_conflict(db, owner, *, winner_id: str, loser_id: str, resolution: str = "resolved") -> dict:
    """Spec §3: settle a dispute without losing either side's history.

    The adopted memory goes back to ``active``; the rejected one becomes
    ``superseded`` (pointing at the winner, so §3's link explains why it is
    gone) or ``expired`` when the resolution dismissed the pair outright.
    Neither row is deleted, and both get a revision first — §1 forbids
    losing the pre-resolution state.
    """
    winner = db.get(Memory, winner_id)
    loser = db.get(Memory, loser_id)
    for row in (winner, loser):
        if row is None or row.owner_id != owner:
            raise ValueError("Memory not found")
    if winner.id == loser.id:
        raise ValueError("resolution needs two distinct memories")
    db.add(
        MemoryRevision(
            owner_id=owner,
            data={"memory_id": winner.id, "previous": {"id": winner.id, **_trace_data(winner)}},
        )
    )
    db.add(
        MemoryRevision(
            owner_id=owner, data={"memory_id": loser.id, "previous": {"id": loser.id, **_trace_data(loser)}}
        )
    )
    dismissed = str(resolution or "").strip().lower() in ("dismiss", "dismissed", "rejected", "discard")
    winner.lifecycle_state = LIFECYCLE_ACTIVE
    winner.disputed_by_id = None
    # Adopting a memory right now is a human re-activation (§1).
    winner.updated_at = now()
    loser.lifecycle_state = LIFECYCLE_EXPIRED if dismissed else LIFECYCLE_SUPERSEDED
    loser.superseded_by_id = winner.id
    loser.disputed_by_id = None
    _event(db, owner, None, "RESOLVE", loser.id, None, winner.id, str(resolution or "resolved"))
    db.flush()
    # §4: both mirrors move with their columns — the winner's `disputed_by_id`
    # is gone, the loser's `superseded_by_id` now names the winner.
    sync_links(db, winner)
    sync_links(db, loser)
    return {
        "winner": winner.id,
        "loser": loser.id,
        "loser_state": loser.lifecycle_state,
        "resolution": resolution,
    }


def _as_utc(value):
    if isinstance(value, str):
        try:
            value = datetime.fromisoformat(value)
        except ValueError:
            return None
    if not isinstance(value, datetime):
        return None
    return value.replace(tzinfo=UTC) if value.tzinfo is None else value.astimezone(UTC)


def decay(db, owner=None, limit=200) -> int:
    """Spec §3 daily decay: weaken what has not been re-activated, archive at the floor.

    Rules the tests pin down:

    * ``pinned=true`` rows are never touched (§3 exempts them outright);
    * ``updated_at`` is never written — it is the *input* to the decay, so
      bumping it would reset the age being measured;
    * only retrievable rows are candidates, so an already-archived row cannot
      consume the ``limit`` budget forever;
    * values decay exponentially on the anchor of the last application
      (``data['last_decay_at']``, else ``updated_at``): anchor and values are
      written together or not at all, so daily runs compound correctly
      instead of double-decaying a row;
    * rows idle for less than half a day are left alone — a daily job has
      nothing to say about a memory from ten minutes ago.

    Returns the number of rows weakened (archived transitions included, and
    those are the ones logged as ``DECAY`` action events; plain scalar decay
    is a recomputation of §1's own fields, not an edit, so it writes no
    revision and no event).
    """
    statement = (
        select(Memory)
        .where(Memory.lifecycle_state.in_(sorted(LIFECYCLE_RETRIEVABLE)))
        .order_by(Memory.updated_at)
        .limit(limit)
    )
    if owner:
        statement = statement.where(Memory.owner_id == owner)
    touched = 0
    for row in db.scalars(statement):
        if is_pinned(row):
            continue
        anchor = (
            _as_utc((row.data or {}).get("last_decay_at"))
            or _as_utc(row.updated_at)
            or _as_utc(row.created_at)
        )
        if anchor is None:
            continue
        elapsed = (datetime.now(UTC) - anchor).total_seconds() / 86400
        if elapsed < DECAY_MIN_DAYS:
            continue
        data = dict(row.data or {})
        current_strength = number(data.get("strength"), DEFAULT_STRENGTH)
        current_salience = number(row.salience, DEFAULT_SALIENCE)
        factor = 0.5 ** (elapsed / DECAY_HALF_LIFE_DAYS)
        strength = clamp(current_strength * factor)
        salience = clamp(current_salience * factor)
        if strength == current_strength and salience == current_salience:
            continue
        archived = strength <= DECAY_FLOOR or salience <= DECAY_FLOOR
        data["strength"] = strength
        # The anchor and the values move together (see the docstring).
        data["last_decay_at"] = now().isoformat()
        row.data = data
        row.salience = salience
        if archived:
            row.lifecycle_state = LIFECYCLE_ARCHIVED
            _event(
                db, row.owner_id, None, "DECAY", row.id, None, None, f"archived after {elapsed:.0f} days idle"
            )
        touched += 1
    if touched:
        db.flush()
    return touched
