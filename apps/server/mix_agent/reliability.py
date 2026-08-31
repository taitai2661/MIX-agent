"""Availability learning for Auto routing; no provider calls or remote error bodies."""

from __future__ import annotations

import email.utils
import math
from datetime import UTC, datetime, timedelta

import httpx
from sqlalchemy import delete, select

from mix_agent.db.models import AutoReliabilityEvent
from mix_agent.providers.adapters import ProviderContextLimitError, is_context_limit_error

RETENTION = timedelta(days=30)
HALF_LIFE = timedelta(days=7)
MODEL_WINDOW = timedelta(minutes=10)
PROVIDER_WINDOW = timedelta(minutes=5)
MODEL_COOLDOWN = timedelta(minutes=15)
PROVIDER_COOLDOWN = timedelta(minutes=15)
MAX_COOLDOWN = timedelta(minutes=60)


def usage_scope(mode: str, tools_required: bool) -> str:
    return "tool" if tools_required else ("thinking" if mode == "thinking" else "chat")


def classify_failure(exc: Exception) -> str:
    """Classify without retaining provider-provided content."""
    if isinstance(exc, (httpx.TimeoutException, httpx.NetworkError, TimeoutError)):
        return "timeout"
    if is_context_limit_error(exc) or isinstance(exc, ProviderContextLimitError):
        return "context"
    if "tool" in str(exc).casefold() and "support" in str(exc).casefold():
        return "tool"
    response = getattr(exc, "response", None)
    code = getattr(response, "status_code", None) or getattr(exc, "status_code", None)
    if code == 429:
        return "rate_limit"
    if code in {408, 425, 500, 502, 503, 504}:
        return "provider_5xx"
    if code in {401, 403}:
        return "auth"
    if code == 404:
        return "not_found"
    return "other"


def retry_after(exc: Exception, current: datetime | None = None) -> datetime | None:
    response = getattr(exc, "response", None)
    raw = response.headers.get("retry-after") if response is not None else None
    if not raw:
        return None
    current = current or now()
    try:
        seconds = max(0, float(raw))
        return current + timedelta(seconds=seconds)
    except ValueError:
        try:
            parsed = email.utils.parsedate_to_datetime(raw)
            if parsed.tzinfo is None:
                parsed = parsed.replace(tzinfo=UTC)
            return parsed
        except (TypeError, ValueError, IndexError):
            return None


def now() -> datetime:
    return datetime.now(UTC)


def record(db, owner_id: str, model_id: str, provider_id: str, scope: str, outcome: str,
           classification: str | None = None, retry_after_until: datetime | None = None,
           current: datetime | None = None) -> AutoReliabilityEvent:
    current = current or now()
    # Opportunistic bounded retention keeps this independent of a scheduler.
    db.execute(delete(AutoReliabilityEvent).where(
        AutoReliabilityEvent.owner_id == owner_id,
        AutoReliabilityEvent.created_at < current - RETENTION,
    ))
    event = AutoReliabilityEvent(owner_id=owner_id, data={
        "model_id": model_id, "provider_id": provider_id, "scope": scope,
        "outcome": outcome, "classification": classification,
        "retry_after_until": retry_after_until.isoformat() if retry_after_until else None,
    }, created_at=current)
    db.add(event)
    return event


def _events(db, owner_id: str, current: datetime):
    return list(db.scalars(select(AutoReliabilityEvent).where(
        AutoReliabilityEvent.owner_id == owner_id,
        AutoReliabilityEvent.created_at >= current - RETENTION,
    )))


def _weight(created_at: datetime, current: datetime) -> float:
    if created_at.tzinfo is None:
        created_at = created_at.replace(tzinfo=UTC)
    age = max(0.0, (current - created_at).total_seconds())
    return math.exp(-math.log(2) * age / HALF_LIFE.total_seconds())


def _rate(events, current: datetime) -> tuple[float, float]:
    success = failure = 0.0
    for event in events:
        weight = _weight(event.created_at, current)
        if event.data.get("outcome") == "success":
            success += weight
        elif event.data.get("classification") in {"rate_limit", "provider_5xx", "timeout"}:
            failure += weight
    return success, failure


def _cooldowns(events, model_id: str, provider_id: str, scope: str, current: datetime):
    def timestamp(event):
        return event.created_at if event.created_at.tzinfo else event.created_at.replace(tzinfo=UTC)
    model_successes = [timestamp(e) for e in events if e.data.get("model_id") == model_id
                       and e.data.get("scope") == scope and e.data.get("outcome") == "success"]
    model_last_success = max(model_successes, default=None)
    model_failures = [e for e in events if e.data.get("model_id") == model_id
                      and e.data.get("scope") == scope
                      and e.data.get("classification") in {"rate_limit", "provider_5xx", "timeout"}
                      and timestamp(e) >= current - MODEL_WINDOW
                      and (model_last_success is None or timestamp(e) > model_last_success)]
    model_until = None
    if len(model_failures) >= 2:
        duration = min(MAX_COOLDOWN, MODEL_COOLDOWN * (2 ** (len(model_failures) - 2)))
        model_until = max(timestamp(e) for e in model_failures) + duration
    provider_successes = [timestamp(e) for e in events if e.data.get("provider_id") == provider_id
                          and e.data.get("scope") == scope and e.data.get("outcome") == "success"]
    provider_last_success = max(provider_successes, default=None)
    provider_failures = [e for e in events if e.data.get("provider_id") == provider_id
                         and e.data.get("scope") == scope
                         and e.data.get("classification") in {"provider_5xx", "timeout"}
                         and timestamp(e) >= current - PROVIDER_WINDOW
                         and (provider_last_success is None or timestamp(e) > provider_last_success)]
    provider_until = None
    if len({e.data.get("model_id") for e in provider_failures}) >= 3:
        provider_until = max(timestamp(e) for e in provider_failures) + PROVIDER_COOLDOWN
    for event in events:
        if (event.data.get("provider_id") != provider_id or event.data.get("scope") != scope
                or event.data.get("classification") != "rate_limit"):
            continue
        raw = event.data.get("retry_after_until")
        if raw:
            try:
                until = datetime.fromisoformat(raw)
                if until.tzinfo is None:
                    until = until.replace(tzinfo=UTC)
                if until > current:
                    provider_until = max(provider_until, until) if provider_until else until
            except ValueError:
                pass
    return model_until if model_until and model_until > current else None, provider_until if provider_until and provider_until > current else None


def reliability(db, owner_id: str, model_id: str, provider_id: str, scope: str,
                current: datetime | None = None) -> dict:
    current = current or now()
    events = _events(db, owner_id, current)
    # Hierarchical, decayed evidence. Small direct samples are deliberately shrunk
    # toward broader model/provider evidence and a conservative beta prior.
    groups = (
        (4.0, [e for e in events if e.data.get("model_id") == model_id and e.data.get("provider_id") == provider_id and e.data.get("scope") == scope]),
        (2.0, [e for e in events if e.data.get("model_id") == model_id]),
        (1.0, [e for e in events if e.data.get("provider_id") == provider_id and e.data.get("scope") == scope]),
        (0.5, [e for e in events if e.data.get("provider_id") == provider_id]),
    )
    success, failure = 8.0, 1.0
    for multiplier, group in groups:
        up, down = _rate(group, current)
        success += multiplier * up
        failure += multiplier * down
    probability = success / (success + failure)
    model_until, provider_until = _cooldowns(events, model_id, provider_id, scope, current)
    penalty = (1 - probability) * 0.30
    reasons = []
    if model_until:
        penalty += 0.70
        reasons.append("最近このモデルで失敗が続いたため一時回避")
    if provider_until:
        penalty += 0.70
        reasons.append("プロバイダが一時的に不安定（クールダウン中）")
    return {"penalty": penalty, "success_probability": probability, "model_cooldown_until": model_until,
            "provider_cooldown_until": provider_until, "reason": "、".join(reasons) if reasons else None}
