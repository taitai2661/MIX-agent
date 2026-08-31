"""Local, deterministic Auto model routing.  This module never contacts providers."""

import hashlib
import math
import re
from collections import defaultdict

from sqlalchemy import select

from mix_agent.db.models import Feedback, Model
from mix_agent.providers.model_roles import is_auto_chat_eligible
from mix_agent.reliability import reliability, usage_scope

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


def routing_profile(content, mode, artifact_mimes, tools_required):
    """A deliberately small, stable, non-user-visible feature key."""
    features = ["coding" if CODING.search(content) else "general"]
    if REASONING.search(content) or mode == "thinking":
        features.append("reasoning")
    if tools_required:
        features.append("tools")
    if any(mime.startswith("image/") for mime in artifact_mimes):
        features.append("vision")
    return ":".join(features)


def estimate_tokens(parts, attachment_bytes=0):
    # UTF-8/2 is intentionally conservative for Japanese, code and mixed input.
    return sum(math.ceil(len((part or "").encode("utf-8")) / 2) for part in parts) + math.ceil(attachment_bytes / 2)


def _scores(db, owner_id, profile):
    results = defaultdict(lambda: [0, 0])
    for item in db.scalars(select(Feedback).where(Feedback.owner_id == owner_id)):
        data = item.data
        if data.get("profile") != profile:
            continue
        if data.get("value") == "up":
            results[data["model_id"]][0] += 1
        elif data.get("value") == "down":
            results[data["model_id"]][1] += 1
    return results


def select_auto_model(db, owner_id, allowed_ids, content, mode, artifact_mimes, tools_required,
                      context_parts, reserved_output_tokens, request_key, attachment_bytes=0,
                      excluded_model_ids=(), prefer_other_provider_than=None):
    """Return a selected Model plus auditable local routing details, or a reason."""
    profile = routing_profile(content, mode, artifact_mimes, tools_required)
    required_tokens = estimate_tokens(context_parts, attachment_bytes) + reserved_output_tokens
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
    reliability_details = {}
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
        health = reliability(db, owner_id, model.id, model.data.get("provider_id"), scope)
        reliability_details[model.id] = health
        provider_retry_penalty = 1.0 if has_other_provider and model.data.get("provider_id") == prefer_other_provider_than else 0
        ranked.append((mean + exploration + reasoning_bonus - context_penalty - health["penalty"] - provider_retry_penalty,
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
        "reliability": {
            "scope": scope,
            "success_probability": selected_health["success_probability"],
            "model_cooldown": bool(selected_health["model_cooldown_until"]),
            "provider_cooldown": bool(selected_health["provider_cooldown_until"]),
            "summary": selected_health["reason"] or (avoided[0] if avoided else None),
        },
        "reason": "。".join(reason_parts),
    }
