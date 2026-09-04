"""Local, deterministic Auto model routing.  This module never contacts providers."""

import hashlib
import math
import re
from collections import defaultdict

from sqlalchemy import select

from mix_agent.db.models import Feedback, Model
from mix_agent.providers.model_roles import is_auto_chat_eligible
from mix_agent.reliability import _events, context_failure_ceiling, now, reliability, speed, usage_scope

CODING = re.compile(r"```|\b(?:python|javascript|typescript|react|sql|docker|api|bug|code|コード|実装|修正)\b", re.IGNORECASE)
REASONING = re.compile(r"\b(?:why|prove|derive|analy[sz]e|reason|比較|理由|証明|推論|考察|複雑)\b", re.IGNORECASE)


def effective_capabilities(data):
    caps = dict(data.get("capabilities", {}))
    caps.update(data.get("overrides", {}))
    if caps.get("tools") is None:
        caps["tools"] = {"supported": True, "unsupported": False}.get(
            data.get("tool_probe", {}).get("status")
        )
    return caps


def routing_profile(content, mode, artifact_mimes, tools_required, required_tokens=None):
    """A deliberately small, stable, non-user-visible feature key."""
    features = ["coding" if CODING.search(content) else "general"]
    if REASONING.search(content) or mode == "thinking":
        features.append("reasoning")
    if tools_required:
        features.append("tools")
    if any(mime.startswith("image/") for mime in artifact_mimes):
        features.append("vision")
    if required_tokens is not None:
        features.append("input:" + ("small" if required_tokens <= 4_096 else "medium" if required_tokens <= 32_768 else "large"))
    return ":".join(features)


def estimate_tokens(parts, attachment_bytes=0, model_id=""):
    from mix_agent.context import budget as context_budget

    return context_budget.estimate_for_routing(list(parts or []), attachment_bytes, model_id)


def _scores(db, owner_id, profile):
    results = defaultdict(lambda: [0, 0])
    for item in db.scalars(select(Feedback).where(Feedback.owner_id == owner_id)):
        data = item.data
        # Existing feedback predates input-size profiles.  Keep it usable as a
        # migration-only fallback; newly created records always use the exact key.
        legacy_profile = profile.rsplit(":input:", 1)[0]
        if data.get("profile") not in {profile, legacy_profile}:
            continue
        if data.get("value") == "up":
            results[data["model_id"]][0] += 1
        elif data.get("value") == "down":
            results[data["model_id"]][1] += 1
    return results


def _speed_scores(details):
    """Convert comparable timing evidence into a bounded routing signal."""
    result = {model_id: 0.0 for model_id in details}
    for value_key, evidence_key, maximum, eligible in (
        ("first_output_ms", "first_output_evidence", 0.10, lambda item: True),
        ("completion_value", "completion_evidence", 0.10, lambda item: item["completion_normalized"]),
        # Raw completion time is only comparable with other providers that also
        # lack token usage; never compare milliseconds with milliseconds/token.
        ("completion_value", "completion_evidence", 0.06, lambda item: not item["completion_normalized"]),
    ):
        known = [item[value_key] for item in details.values()
                 if item[value_key] is not None and item[evidence_key] >= 3.0 and eligible(item)]
        if len(known) < 2:
            continue
        reference = sorted(known)[len(known) // 2]
        for model_id, item in details.items():
            value = item[value_key]
            if value is None or value <= 0 or item[evidence_key] < 3.0 or not eligible(item):
                continue
            evidence = min(1.0, item[evidence_key] / 3.0)
            # Log comparison resists one pathological slow sample dominating.
            result[model_id] += maximum * evidence * max(-1.0, min(1.0, math.log(reference / value)))
    return result


def select_auto_model(db, owner_id, allowed_ids, content, mode, artifact_mimes, tools_required,
                      context_parts, reserved_output_tokens, request_key, attachment_bytes=0,
                      excluded_model_ids=(), prefer_other_provider_than=None):
    """Return a selected Model plus auditable local routing details, or a reason."""
    required_tokens = estimate_tokens(context_parts, attachment_bytes) + reserved_output_tokens
    profile = routing_profile(content, mode, artifact_mimes, tools_required, required_tokens)
    candidates, excluded, skipped = [], defaultdict(int), set(excluded_model_ids)
    for model in db.scalars(select(Model).where(Model.owner_id == owner_id, Model.id.in_(allowed_ids))):
        data, caps = model.data, effective_capabilities(model.data)
        if model.id in skipped:
            continue
        if not is_auto_chat_eligible(data):
            excluded["chat"] += 1
            continue
        context_window = data.get("context_window")
        if context_window and required_tokens > context_window:
            excluded["context"] += 1
            continue
        # API-provided and manual windows above are authoritative.  For unknown
        # windows only, avoid retrying a request size already proven to fail.
        learned_ceiling = context_failure_ceiling(db, owner_id, model.id) if not context_window else None
        if learned_ceiling and required_tokens >= learned_ceiling:
            excluded["context"] += 1
            continue
        if any(mime.startswith("image/") for mime in artifact_mimes) and caps.get("vision") is not True:
            excluded["vision"] += 1
            continue
        if tools_required and caps.get("tools") is not True:
            excluded["tools"] += 1
            continue
        candidates.append((model, caps, context_window))
    if not candidates:
        details = "、".join({"chat": "通常チャット", "context": "Context Window", "vision": "Vision", "tools": "Tool Calling"}[k] for k in excluded)
        reason = "Autoで使用可能なモデルに必要な " + (details or "条件") + " を満たすものがありません。"
        return None, {"profile": profile, "candidate_count": 0, "required_tokens": required_tokens,
                      "reason": reason}

    outcomes = _scores(db, owner_id, profile)
    total = sum(up + down for up, down in outcomes.values())
    ranked = []
    context_usage = {}
    scope = usage_scope(mode, tools_required)
    events = _events(db, owner_id, now())
    reliability_details = {}
    speed_details = {}
    has_other_provider = prefer_other_provider_than and any(
        model.data.get("provider_id") != prefer_other_provider_than for model, _, _ in candidates
    )
    for model, caps, context_window in candidates:
        up, down = outcomes[model.id]
        count = up + down
        # Strong Beta(8, 8) prior prevents a single rating from dominating.
        mean = (up + 8) / (count + 16)
        exploration = 0.35 * math.sqrt(
            2 * math.log(total + 16 * len(candidates) + 1) / (count + 16)
        )
        reasoning_bonus = 0.04 if mode == "thinking" and caps.get("reasoning") is True else 0
        usage = required_tokens / context_window if context_window else None
        # Known models above 50% capacity remain available, but increasingly
        # lose preference to models with more demonstrated context headroom.
        context_penalty = max(0, usage - 0.5) * 0.1 if usage is not None else 0
        context_usage[model.id] = {
            "window": context_window,
            "usage_ratio": usage,
            "penalty": context_penalty,
        }
        health = reliability(db, owner_id, model.id, model.data.get("provider_id"), scope, profile=profile, events=events)
        reliability_details[model.id] = health
        speed_details[model.id] = speed(db, owner_id, model.id, model.data.get("provider_id"), scope, profile=profile, events=events)
    speed_scores = _speed_scores(speed_details)
    for model, caps, context_window in candidates:
        up, down = outcomes[model.id]
        count = up + down
        mean = (up + 8) / (count + 16)
        exploration = 0.35 * math.sqrt(
            2 * math.log(total + 16 * len(candidates) + 1) / (count + 16)
        )
        reasoning_bonus = 0.04 if mode == "thinking" and caps.get("reasoning") is True else 0
        usage = required_tokens / context_window if context_window else None
        context_penalty = max(0, usage - 0.5) * 0.1 if usage is not None else 0
        health = reliability_details[model.id]
        provider_retry_penalty = 1.0 if has_other_provider and model.data.get("provider_id") == prefer_other_provider_than else 0
        ranked.append((mean + exploration + reasoning_bonus + speed_scores[model.id] - context_penalty - health["penalty"] - provider_retry_penalty,
                       model, up, down))
    best = max(score for score, *_ in ranked)
    tied = [entry for entry in ranked if abs(entry[0] - best) < 1e-12]
    # Stable per request, while naturally distributing exact ties across requests.
    tied.sort(key=lambda entry: hashlib.sha256((request_key + entry[1].id).encode()).hexdigest())
    _, selected, up, down = tied[0]
    selected_health = reliability_details[selected.id]
    reason_parts = ["Auto: " + profile + " の実績・探索度・既知Contextの余裕から選択"]
    if selected_health["reason"]:
        reason_parts.append(selected_health["reason"])
    if speed_details[selected.id]["first_output_ms"] is not None or speed_details[selected.id]["completion_value"] is not None:
        reason_parts.append("応答速度の実績も考慮")
    avoided = [item["reason"] for key, item in reliability_details.items()
               if key != selected.id and item["reason"]]
    if avoided and not selected_health["reason"]:
        reason_parts.append(avoided[0])
    return selected, {
        "profile": profile,
        "candidate_count": len(candidates),
        "candidate_ids": [model.id for model, _, _ in candidates],
        "required_tokens": required_tokens,
        "context_usage": context_usage,
        "feedback": {"up": up, "down": down},
        "speed": {"score": speed_scores[selected.id], **speed_details[selected.id]},
        "reliability": {
            "scope": scope,
            "success_probability": selected_health["success_probability"],
            "model_cooldown": bool(selected_health["model_cooldown_until"]),
            "provider_cooldown": bool(selected_health["provider_cooldown_until"]),
            "summary": selected_health["reason"] or (avoided[0] if avoided else None),
        },
        "reason": "。".join(reason_parts),
    }
