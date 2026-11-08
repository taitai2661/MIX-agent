"""Spec §3 lifecycle: restore, reinforcement, conflict resolution and decay."""

from __future__ import annotations

from datetime import UTC, datetime

from sqlalchemy import select

from mix_agent.db.models import Memory, MemoryRevision, now
from mix_agent.memory.common import (
    DECAY_FLOOR,
    DECAY_HALF_LIFE_DAYS,
    DECAY_MIN_DAYS,
    DEFAULT_CONFIDENCE,
    DEFAULT_SALIENCE,
    DEFAULT_STRENGTH,
    REINFORCE_CONFIDENCE_STEP,
    REINFORCE_SALIENCE_STEP,
    _event,
    _trace_data,
    clamp,
    is_pinned,
    lifecycle_value,
    number,
    role_value,
    scope_value,
    source_kind_value,
)
from mix_agent.memory.links import sync_links
from mix_agent.memory.types import (
    LIFECYCLE_ACTIVE,
    LIFECYCLE_ARCHIVED,
    LIFECYCLE_EXPIRED,
    LIFECYCLE_RETRIEVABLE,
    LIFECYCLE_SUPERSEDED,
)


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
