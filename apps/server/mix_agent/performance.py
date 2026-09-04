"""Privacy-minimal completed-answer throughput measurements."""

from datetime import timedelta

from sqlalchemy import delete

from mix_agent.db.models import PerformanceEvent, now

RETENTION = timedelta(days=30)


def record(db, owner_id: str, model_id: str, provider_id: str, mode: str,
           output_tokens: int, generation_ms: int) -> PerformanceEvent:
    """Store only the values needed to compare completed answer throughput."""
    current = now()
    db.execute(delete(PerformanceEvent).where(
        PerformanceEvent.owner_id == owner_id,
        PerformanceEvent.created_at < current - RETENTION,
    ))
    return PerformanceEvent(owner_id=owner_id, data={
        "model_id": model_id,
        "provider_id": provider_id,
        "mode": mode,
        "output_tokens": output_tokens,
        "generation_ms": generation_ms,
        "tokens_per_second": round(output_tokens / max(0.001, generation_ms / 1000), 1),
    }, created_at=current)
