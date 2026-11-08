"""Shared Memory vocabulary, constants and row helpers.

Split out of ``service.py`` so ranking, link generation and lifecycle code can
share them without importing the service module back (which would be circular).
"""

from __future__ import annotations

import logging
import re


from mix_agent.db.models import Memory, MemoryActionEvent
from mix_agent.memory.types import (
    LEGACY_LIFECYCLE_MAP,
    LIFECYCLE_ESTABLISHED,
    LIFECYCLE_RETRIEVABLE,
    LIFECYCLES,
    ROLE_FACT,
    ROLES,
    SCOPE_USER,
    SCOPES,
    SOURCE_AGENT,
    SOURCES,
)

LOGGER = logging.getLogger(__name__)

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
