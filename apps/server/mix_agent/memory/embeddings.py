"""Vector helpers for the hybrid ranking defined in ``docs/memory-spec.md`` §5.

Embedding is the only part of recall that touches the network, and recall runs
inside synchronous request paths (context build, tool dispatch, debug routes).
Rather than thread ``async`` through all of them, callers resolve the *query*
vector first — ``query_vector()`` — and hand it to ``memory.service.search``.

The contract that makes this safe: **a missing vector must reproduce today's
ranking exactly.** That is achieved in two places:

* ``query_vector`` returns ``None`` on every failure (unsupported provider,
  timeout, empty text, provider error) instead of raising;
* ``service.search`` selects the ``without_vector`` weights whenever the pair
  (query, candidate) does not have two usable vectors, so rows that have not
  been embedded yet keep the score they have today.
"""

from __future__ import annotations

import asyncio
import hashlib
import logging
import math
import time
from collections import OrderedDict

from sqlalchemy import select

LOGGER = logging.getLogger(__name__)

# Bound the latency an embedding lookup can add to a request. Exceeding it is
# not an error: the caller ranks lexically.
EMBED_TIMEOUT_SECONDS = 5.0
QUERY_CACHE_TTL_SECONDS = 900.0
QUERY_CACHE_MAX_ENTRIES = 256
BACKFILL_SCAN_LIMIT = 2000
EMBED_BATCH_SIZE = 8

# provider id + normalized query -> (monotonic timestamp, vector)
_query_cache: OrderedDict[str, tuple[float, list[float]]] = OrderedDict()


def row_vector(data) -> list[float] | None:
    """Stored embedding for a Memory row, or ``None`` when unusable."""
    if not isinstance(data, dict):
        return None
    raw = data.get("embedding")
    if not isinstance(raw, list) or len(raw) < 8:
        return None
    try:
        vector = [float(value) for value in raw]
    except (TypeError, ValueError):
        return None
    if any(not math.isfinite(value) for value in vector):
        return None
    return vector


def cosine01(left, right) -> float | None:
    """Cosine similarity mapped to ``[0, 1]``, or ``None`` if it is not defined.

    Mismatched dimensions return ``None`` rather than 0: "these two vectors
    cannot be compared" and "these two are unrelated" are different claims, and
    only the second one should be allowed to lower a candidate's score.
    """
    if not left or not right or len(left) != len(right):
        return None
    dot = 0.0
    norm_left = 0.0
    norm_right = 0.0
    for a, b in zip(left, right):
        if not (math.isfinite(a) and math.isfinite(b)):
            return None
        dot += a * b
        norm_left += a * a
        norm_right += b * b
    if norm_left <= 0.0 or norm_right <= 0.0:
        return None
    similarity = dot / math.sqrt(norm_left * norm_right)
    return max(0.0, min(1.0, (1.0 + similarity) / 2.0))


async def _embed_batch(provider, key, texts) -> tuple[str, list[list[float]]] | None:
    """Core call: ``(model_id, vectors)`` or ``None`` on any failure.

    Every caller turns ``None`` into "rank lexically" or "leave the row
    un-vectored", so a slow or absent embedding endpoint can never fail a
    request, a tool call or a background job.
    """
    items = [str(text).strip() for text in texts or [] if str(text or "").strip()]
    if provider is None or not items:
        return None
    try:
        from mix_agent.providers.adapters import Adapter

        async with asyncio.timeout(EMBED_TIMEOUT_SECONDS):
            adapter = Adapter(provider, key)
            model = adapter.embedding_model()
            vectors = await adapter.embed(items)
    except TimeoutError:
        LOGGER.debug("embedding timed out provider=%s", provider.get("kind"))
        return None
    except Exception as exc:  # noqa: BLE001 - a ranking fallback must never surface an error
        LOGGER.debug("embedding unavailable provider=%s error=%s", provider.get("kind"), type(exc).__name__)
        return None
    if not vectors or len(vectors) != len(items):
        return None
    return (str(model or ""), vectors)


async def embed_texts(provider, key, texts) -> list[list[float]] | None:
    """Embed ``texts``; ``None`` means "treat this as no embedding at all"."""
    batch = await _embed_batch(provider, key, texts)
    return batch[1] if batch else None


async def embed_content(provider, key, text) -> tuple[str, list[float]] | None:
    """Embed one string, returning ``(model_id, vector)``; ``None`` on failure.

    The model id travels with the vector so a provider switch can be detected
    and the stale vector replaced rather than mixed into a second space.
    """
    batch = await _embed_batch(provider, key, [text])
    if not batch:
        return None
    return (batch[0], batch[1][0])


def _cache_key(provider, text: str) -> str:
    provider_id = str(provider.get("id") if isinstance(provider, dict) else provider or "")
    return hashlib.sha256(provider_id.encode() + b"\x00" + text.encode()).hexdigest()


async def query_vector(provider, key, query) -> list[float] | None:
    """Vector for ``query``, cached. ``None`` on any failure or blank input."""
    text = " ".join(str(query or "").split())
    if len(text) < 2 or provider is None:
        return None
    cache_key = _cache_key(provider, text)
    now = time.monotonic()
    cached = _query_cache.get(cache_key)
    if cached is not None and now - cached[0] < QUERY_CACHE_TTL_SECONDS:
        _query_cache.move_to_end(cache_key)
        return cached[1]
    vectors = await embed_texts(provider, key, [text])
    vector = vectors[0] if vectors else None
    if vector is None:
        return None
    _query_cache[cache_key] = (now, vector)
    _query_cache.move_to_end(cache_key)
    while len(_query_cache) > QUERY_CACHE_MAX_ENTRIES:
        _query_cache.popitem(last=False)
    return vector


def store(data, model, vector) -> dict:
    """New ``data`` payload carrying the embedding and the model that made it.

    The model id travels with the vector so a provider switch invalidates stale
    vectors instead of mixing two incompatible spaces in one ranking.
    """
    updated = dict(data or {})
    updated["embedding"] = [float(value) for value in vector]
    updated["embedding_model"] = str(model or "")
    return updated


def is_stale(data, model) -> bool:
    """True when a row has no vector, or one from a different model."""
    if row_vector(data) is None:
        return True
    if not model:
        return False
    return str((data or {}).get("embedding_model") or "") != str(model)


def default_provider(db, owner_id):
    """Most recent provider record for the owner."""
    from mix_agent.db.models import Provider

    return db.scalar(
        select(Provider).where(Provider.owner_id == owner_id).order_by(Provider.created_at.desc()).limit(1)
    )


async def embed_memory(db, memory_id: str, provider=None, key=None) -> bool:
    """Give one Memory row its vector. Returns whether it changed.

    An embedding is *derived* data: it is written straight into ``data`` with
    no revision, no action event and no ``updated_at`` bump. That keeps the
    revision trail free of index noise and stops a re-embed from quietly
    inflating the recency term of the very ranking it feeds.
    """
    from mix_agent.auth.security import read_secret
    from mix_agent.db.models import Memory

    row = db.get(Memory, memory_id)
    if row is None:
        return False
    content = str((row.data or {}).get("content") or row.content or "").strip()
    if not content:
        return False
    if provider is None:
        provider = default_provider(db, row.owner_id)
        key = read_secret(db, provider.data.get("secret_id")) if provider else None
    if provider is None:
        return False
    embedded = await embed_content({**provider.data, "id": provider.id}, key, content)
    if embedded is None or not is_stale(row.data, embedded[0]):
        return False
    row.data = store(dict(row.data or {}), embedded[0], embedded[1])
    return True


async def embed_contents(db, memory_ids, provider=None, key=None) -> int:
    """Embed several rows in one provider call; returns how many changed.

    Auto-extraction writes a handful of memories at once, so one request for
    the whole batch rather than one per row keeps a background job's latency
    proportional to the batch, not to the number of rows in it.
    """
    from mix_agent.auth.security import read_secret
    from mix_agent.db.models import Memory

    rows = [
        row
        for row in (db.get(Memory, mid) for mid in list(memory_ids or [])[:EMBED_BATCH_SIZE])
        if row is not None
    ]
    if not rows:
        return 0
    if provider is None:
        provider = default_provider(db, rows[0].owner_id)
        key = read_secret(db, provider.data.get("secret_id")) if provider else None
    if provider is None:
        return 0
    # Only rows whose text has no vector yet, so a retry never redoes the work.
    pending = [row for row in rows if row_vector(row.data) is None]
    texts = [str((row.data or {}).get("content") or row.content or "").strip() for row in pending]
    keep = [(row, text) for row, text in zip(pending, texts) if text]
    if not keep:
        return 0
    batch = await _embed_batch({**provider.data, "id": provider.id}, key, [text for _, text in keep])
    if batch is None:
        return 0
    for (row, _), vector in zip(keep, batch[1]):
        row.data = store(dict(row.data or {}), batch[0], vector)
    return len(keep)


async def embed_pending(db, owner_id: str, provider=None, key=None, limit: int = 50) -> int:
    """Backfill vectors for rows that still have none; returns how many landed."""
    from mix_agent.auth.security import read_secret
    from mix_agent.db.models import Memory

    if provider is None:
        provider = default_provider(db, owner_id)
        key = read_secret(db, provider.data.get("secret_id")) if provider else None
    if provider is None:
        return 0
    wire_provider = {**provider.data, "id": provider.id}
    rows = db.scalars(
        select(Memory)
        .where(Memory.owner_id == owner_id)
        .order_by(Memory.updated_at.desc())
        .limit(BACKFILL_SCAN_LIMIT)
    ).all()
    done = 0
    for row in rows:
        if done >= limit:
            break
        if row_vector(row.data) is not None:
            continue
        content = str((row.data or {}).get("content") or row.content or "").strip()
        if not content:
            continue
        embedded = await embed_content(wire_provider, key, content)
        if embedded is None:
            break  # the provider cannot embed at all — not a per-row failure
        row.data = store(dict(row.data or {}), embedded[0], embedded[1])
        done += 1
    db.commit()
    return done


def clear_query_cache() -> None:
    _query_cache.clear()
