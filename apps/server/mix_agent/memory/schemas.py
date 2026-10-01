"""Lightweight validators for role-specific Memory metadata blocks.

These schemas are intentionally lenient (no third-party dependency, no
pydantic round-trip): the agent runtime needs to persist and read each
memory shape without crashing on partial data.  Failures degrade to
``None`` so the caller can fall back to generic content rendering.
"""

from __future__ import annotations

from typing import Any


def _clip(value: Any, limit: int) -> Any:
    if isinstance(value, str):
        return value[:limit]
    if isinstance(value, list):
        return [str(item)[: limit // 4] for item in value[:20]]
    return value


def validate_decision_metadata(raw: Any) -> dict | None:
    """Decision Memory: keep the why, the alternatives and the source.

    Fields:
      - decision: short label of the decision itself
      - reason: why the decision was made
      - alternatives: considered alternatives
      - rejected_alternatives: considered but rejected, with reason
      - source: who/what made the decision
      - timestamp: ISO8601 string (optional)
      - scope / status: carried through for downstream rendering
    """
    if not isinstance(raw, dict):
        return None
    out: dict[str, Any] = {}
    if isinstance(raw.get("decision"), str):
        out["decision"] = raw["decision"][:300]
    if isinstance(raw.get("reason"), str):
        out["reason"] = raw["reason"][:2000]
    if isinstance(raw.get("alternatives"), list):
        out["alternatives"] = [str(a)[:200] for a in raw["alternatives"][:20]]
    if isinstance(raw.get("rejected_alternatives"), list):
        rejected = []
        for entry in raw["rejected_alternatives"][:20]:
            if isinstance(entry, str):
                rejected.append({"option": entry[:200], "reason": ""})
            elif isinstance(entry, dict):
                rejected.append({
                    "option": str(entry.get("option", ""))[:200],
                    "reason": str(entry.get("reason", ""))[:500],
                })
        out["rejected_alternatives"] = rejected
    if isinstance(raw.get("source"), str):
        out["source"] = raw["source"][:200]
    if isinstance(raw.get("timestamp"), str):
        out["timestamp"] = raw["timestamp"][:40]
    if isinstance(raw.get("status"), str):
        out["status"] = raw["status"][:40]
    return out or None


def validate_failure_metadata(raw: Any) -> dict | None:
    """Failure Memory: keep the attempt, the cause, the lesson and the evidence.

    Fields:
      - attempt: what was tried
      - outcome: 'failed' | 'partial' | 'blocked'
      - reason: why it failed
      - lesson: the lesson the agent should remember
      - environment: conditions under which it failed (e.g. OS, version)
      - related_tool: which tool produced the failure (if any)
      - retry_suggested: bool
    """
    if not isinstance(raw, dict):
        return None
    out: dict[str, Any] = {}
    if isinstance(raw.get("attempt"), str):
        out["attempt"] = raw["attempt"][:1000]
    outcome = raw.get("outcome")
    if outcome in {"failed", "partial", "blocked", "succeeded"}:
        out["outcome"] = outcome
    if isinstance(raw.get("reason"), str):
        out["reason"] = raw["reason"][:2000]
    if isinstance(raw.get("lesson"), str):
        out["lesson"] = raw["lesson"][:2000]
    if isinstance(raw.get("environment"), dict):
        env = {str(k)[:80]: str(v)[:200] for k, v in list(raw["environment"].items())[:20]}
        out["environment"] = env
    elif isinstance(raw.get("environment"), str):
        out["environment"] = raw["environment"][:500]
    if isinstance(raw.get("related_tool"), str):
        out["related_tool"] = raw["related_tool"][:100]
    if isinstance(raw.get("retry_suggested"), bool):
        out["retry_suggested"] = raw["retry_suggested"]
    return out or None


def validate_evidence_payload(raw: Any) -> dict | None:
    """Validate an evidence record before persistence."""
    if not isinstance(raw, dict):
        return None
    kind = raw.get("kind")
    if not isinstance(kind, str):
        return None
    out: dict[str, Any] = {"kind": kind[:40]}
    if isinstance(raw.get("ref"), str):
        out["ref"] = raw["ref"][:480]
    if isinstance(raw.get("summary"), str):
        out["summary"] = raw["summary"][:2000]
    if isinstance(raw.get("captured_at"), str):
        out["captured_at"] = raw["captured_at"][:40]
    for opt in ("tool_id", "call_id", "url", "path"):
        if isinstance(raw.get(opt), str):
            out[opt] = raw[opt][:200]
    if isinstance(raw.get("confidence"), (int, float)):
        out["confidence"] = max(0.0, min(1.0, float(raw["confidence"])))
    return out
