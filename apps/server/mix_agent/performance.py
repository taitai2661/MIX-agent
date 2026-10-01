"""Privacy-minimal completed-answer throughput measurements."""

from datetime import timedelta

from sqlalchemy import delete

from mix_agent.db.models import PerformanceEvent, now

RETENTION = timedelta(days=30)


def record(db, owner_id: str, model_id: str, provider_id: str, mode: str,
           output_tokens: int, generation_ms: int,
           first_output_ms: int | None = None) -> PerformanceEvent:
    """Store only the values needed to compare completed answer throughput."""
    current = now()
    db.execute(delete(PerformanceEvent).where(
        PerformanceEvent.owner_id == owner_id,
        PerformanceEvent.created_at < current - RETENTION,
    ))
    data = {
        "model_id": model_id,
        "provider_id": provider_id,
        "mode": mode,
        "output_tokens": output_tokens,
        "generation_ms": generation_ms,
        "tokens_per_second": round(output_tokens / max(0.001, generation_ms / 1000), 1),
    }
    # Time-to-first-token isolates provider queueing from streaming throughput.
    if first_output_ms is not None:
        data["first_output_ms"] = first_output_ms
    return PerformanceEvent(owner_id=owner_id, data=data, created_at=current)
