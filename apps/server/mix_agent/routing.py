"""Local, deterministic Auto model routing.  This module never contacts providers."""

import hashlib
import math
import random
import re
from collections import defaultdict

from sqlalchemy import select

from mix_agent.db.models import Feedback, Model
from mix_agent.providers import capability_probe
from mix_agent.providers.model_quality import quality_prior, request_cost
from mix_agent.providers.model_roles import is_auto_chat_eligible
from mix_agent.reliability import _events, context_failure_ceiling, now, reliability, speed, usage_scope

CODING = re.compile(r"```|\b(?:python|javascript|typescript|react|sql|docker|api|bug|code|コード|実装|修正)\b", re.IGNORECASE)
REASONING = re.compile(r"\b(?:why|prove|derive|analy[sz]e|reason|比較|理由|証明|推論|考察|複雑)\b", re.IGNORECASE)
TRANSLATION = re.compile(r"\b(?:translate|translation|英訳|和訳|翻訳)\b", re.IGNORECASE)
SUMMARIZE = re.compile(r"\b(?:summar(?:y|ize)|要約|まとめ)\b", re.IGNORECASE)
EXTRACTION = re.compile(r"\b(?:json|yaml|schema|extract|構造化|抽出|表形式)\b", re.IGNORECASE)
MATH = re.compile(r"\b(?:calculate|compute|equation|integral|derivative|math|数学|計算|解け)\b|[0-9]\s*[+\-*/^]\s*[0-9]", re.IGNORECASE)
JAPANESE = re.compile(r"[\u3040-\u30ff\u4e00-\u9fff]")

# User-selectable tradeoffs.  ``quality_strength`` seeds the Beta prior from the
# static model tier, ``speed_scale`` multiplies learned timing signals,
# ``cost_weight`` scales the cheapness signal, and ``explore`` is the UCB
# exploration constant.  Unknown values fall back to ``balanced``.
PRIORITIES = {
    "balanced": {"quality_strength": 4.0, "speed_scale": 1.0, "cost_weight": 0.03, "explore": 0.35},
    "quality": {"quality_strength": 8.0, "speed_scale": 0.6, "cost_weight": 0.00, "explore": 0.35},
    "speed": {"quality_strength": 2.0, "speed_scale": 1.8, "cost_weight": 0.02, "explore": 0.30},
    "cost": {"quality_strength": 2.0, "speed_scale": 0.6, "cost_weight": 0.18, "explore": 0.30},
}

# Feedback recorded before richer profiles existed still applies through the
# hierarchy: exact route > coarse profile > legacy profile-only key.
LEVEL_WEIGHTS = {"exact": 1.0, "coarse": 0.5, "legacy": 0.25}


class RouteProfile:
    """A stable feature key plus its broader ancestors for evidence reuse."""

    __slots__ = ("coarse", "exact", "legacy")

    def __init__(self, exact, coarse, legacy):
        self.exact = exact
        self.coarse = coarse
        self.legacy = legacy

    @property
    def levels(self):
        levels = {}
        for weight, value in (
            (LEVEL_WEIGHTS["legacy"], self.legacy),
            (LEVEL_WEIGHTS["coarse"], self.coarse),
            (LEVEL_WEIGHTS["exact"], self.exact),
        ):
            if value:
                levels[value] = max(levels.get(value, 0.0), weight)
        return levels

    @property
    def accepted(self):
        return frozenset(self.levels)


def effective_capabilities(data):
    """Effective capabilities with manual overrides and confirmed probes.

    ``capability_probe`` owns the precedence (manual > provider > probe) so the
    same verdicts are used everywhere a model's support is checked.
    """
    return capability_probe.capabilities(data)


def _task_type(content):
    if CODING.search(content):
        return "coding"
    if TRANSLATION.search(content):
        return "translation"
    if SUMMARIZE.search(content):
        return "summarize"
    if EXTRACTION.search(content):
        return "extraction"
    if MATH.search(content):
        return "math"
    if REASONING.search(content):
        return "reasoning"
    return "general"


def _base_tokens(content, mode, artifact_mimes, tools_required, required_tokens):
    tokens = ["coding" if CODING.search(content) else "general"]
    if REASONING.search(content) or mode == "thinking":
        tokens.append("reasoning")
    if tools_required:
        tokens.append("tools")
    if any(mime.startswith("image/") for mime in artifact_mimes):
        tokens.append("vision")
    if required_tokens is not None:
        tokens.append("input:" + ("small" if required_tokens <= 4_096 else "medium" if required_tokens <= 32_768 else "large"))
    return tokens


def routing_context(content, mode, artifact_mimes, tools_required, required_tokens=None):
    """Return the exact feature key alongside its reusable coarse ancestors."""
    content = content or ""
    tokens = _base_tokens(content, mode, artifact_mimes, tools_required, required_tokens)
    size = tokens[-1].removeprefix("input:") if tokens[-1].startswith("input:") else "small"
    task = _task_type(content)
    if size == "large" or task in {"reasoning", "math"}:
        difficulty = "high"
    elif size == "medium" or task == "coding":
        difficulty = "mid"
    else:
        difficulty = "low"
    extra = ["lang:" + ("ja" if JAPANESE.search(content) else "en"), "task:" + task, "diff:" + difficulty]
    coarse = ":".join(tokens)
    legacy = ":".join(token for token in tokens if not token.startswith("input:"))
    return RouteProfile(exact=":".join([*tokens, *extra]), coarse=coarse, legacy=legacy)


def routing_profile(content, mode, artifact_mimes, tools_required, required_tokens=None):
    """A deliberately small, stable, non-user-visible feature key."""
    return routing_context(content, mode, artifact_mimes, tools_required, required_tokens).coarse


def estimate_tokens(parts, attachment_bytes=0, model_id=""):
    from mix_agent.context import budget as context_budget

    return context_budget.estimate_for_routing(list(parts or []), attachment_bytes, model_id)


def _scores(db, owner_id, levels):
    """Weighted feedback counts across the profile hierarchy."""
    results = defaultdict(lambda: [0.0, 0.0])
    for item in db.scalars(select(Feedback).where(Feedback.owner_id == owner_id)):
        data = item.data
        weight = levels.get(data.get("profile"))
        if not weight:
            continue
        if data.get("value") == "up":
            results[data["model_id"]][0] += weight
        elif data.get("value") == "down":
            results[data["model_id"]][1] += weight
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


def _cost_scores(costs):
    """Reward candidates that are cheaper than the comparable median."""
    result = {model_id: 0.0 for model_id in costs}
    known = [cost for cost in costs.values() if cost is not None and cost > 0]
    if len(known) < 2:
        return result
    reference = sorted(known)[len(known) // 2]
    for model_id, cost in costs.items():
        if cost is None or cost <= 0:
            continue
        result[model_id] = max(-1.0, min(1.0, math.log(reference / cost)))
    return result


def _thompson_tiebreak(tied, request_key):
    """Resolve exact score ties by a deterministic per-request Beta draw."""
    def draw(entry):
        _, model, up, down, prior_up, prior_down = entry
        seeded = hashlib.sha256((request_key + model.id).encode()).digest()
        return random.Random(seeded).betavariate(prior_up + up, prior_down + down)

    return sorted(tied, key=lambda entry: (-draw(entry), hashlib.sha256((request_key + entry[1].id).encode()).hexdigest()))


def select_auto_model(db, owner_id, allowed_ids, content, mode, artifact_mimes, tools_required,
                      context_parts, reserved_output_tokens, request_key, attachment_bytes=0,
                      excluded_model_ids=(), prefer_other_provider_than=None, current_model_id=None,
                      priority="balanced"):
    """Return a selected Model plus auditable local routing details, or a reason."""
    weights = PRIORITIES.get(priority, PRIORITIES["balanced"])
    required_tokens = estimate_tokens(context_parts, attachment_bytes) + reserved_output_tokens
    context = routing_context(content, mode, artifact_mimes, tools_required, required_tokens)
    profile = context.exact
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

    outcomes = _scores(db, owner_id, context.levels)
    total = sum(up + down for up, down in outcomes.values())
    ranked = []
    context_usage = {}
    quality_details = {}
    scope = usage_scope(mode, tools_required)
    events = _events(db, owner_id, now())
    reliability_details = {}
    speed_details = {}
    costs = {}
    has_other_provider = prefer_other_provider_than and any(
        model.data.get("provider_id") != prefer_other_provider_than for model, _, _ in candidates
    )
    for model, caps, context_window in candidates:
        quality = quality_prior(model.data)
        quality_details[model.id] = quality
        costs[model.id] = request_cost(model.data, required_tokens, reserved_output_tokens)
        usage = required_tokens / context_window if context_window else None
        context_usage[model.id] = {
            "window": context_window,
            "usage_ratio": usage,
            "penalty": max(0, usage - 0.5) * 0.1 if usage is not None else 0,
        }
        health = reliability(db, owner_id, model.id, model.data.get("provider_id"), scope, profile=context.accepted, events=events)
        reliability_details[model.id] = health
        speed_details[model.id] = speed(db, owner_id, model.id, model.data.get("provider_id"), scope, profile=context.accepted, events=events)
    speed_scores = _speed_scores(speed_details)
    cost_scores = _cost_scores(costs)
    for model, caps, context_window in candidates:
        up, down = outcomes[model.id]
        quality = quality_details[model.id]
        # A known tier becomes a mild Beta pseudo-count prior; unknown stays at
        # the historical Beta(8, 8) so existing behaviour is unchanged.
        if quality is None:
            prior_up = prior_down = 8.0
        else:
            prior_up = 8.0 + weights["quality_strength"] * quality
            prior_down = 8.0 + weights["quality_strength"] * (1.0 - quality)
        count = up + down
        mean = (up + prior_up) / (count + prior_up + prior_down)
        exploration = weights["explore"] * math.sqrt(
            2 * math.log(total + (prior_up + prior_down) * len(candidates) + 1) / (count + prior_up + prior_down)
        )
        reasoning_bonus = 0.04 if mode == "thinking" and caps.get("reasoning") is True else 0
        context_penalty = context_usage[model.id]["penalty"]
        health = reliability_details[model.id]
        provider_retry_penalty = 1.0 if has_other_provider and model.data.get("provider_id") == prefer_other_provider_than else 0
        # Small hysteresis avoids switching on a tie or a marginal score change.
        continuity_bonus = 0.03 if model.id == current_model_id else 0
        score = (
            mean + exploration + reasoning_bonus
            + speed_scores[model.id] * weights["speed_scale"]
            + cost_scores[model.id] * weights["cost_weight"]
            + continuity_bonus - context_penalty - health["penalty"] - provider_retry_penalty
        )
        ranked.append((score, model, up, down, prior_up, prior_down))
    best = max(entry[0] for entry in ranked)
    tied = [entry for entry in ranked if abs(entry[0] - best) < 1e-12]
    # Stable per request: exact ties use a seeded Beta draw, so a candidate with
    # stronger feedback is favoured without introducing cross-request randomness.
    ordered = _thompson_tiebreak(tied, request_key) if len(tied) > 1 else tied
    _, selected, up, down, _, _ = ordered[0]
    selected_health = reliability_details[selected.id]
    reason_parts = ["Auto: " + profile + " (" + priority + ") の実績・探索度・既知Contextの余裕から選択"]
    if selected_health["reason"]:
        reason_parts.append(selected_health["reason"])
    if speed_details[selected.id]["first_output_ms"] is not None or speed_details[selected.id]["completion_value"] is not None:
        reason_parts.append("応答速度の実績も考慮")
    if quality_details[selected.id] is not None:
        reason_parts.append("モデル能力の事前知識も考慮")
    if cost_scores[selected.id]:
        reason_parts.append("コスト効率も考慮")
    avoided = [item["reason"] for key, item in reliability_details.items()
               if key != selected.id and item["reason"]]
    if avoided and not selected_health["reason"]:
        reason_parts.append(avoided[0])
    return selected, {
        "profile": profile,
        "coarse_profile": context.coarse,
        "priority": priority,
        "candidate_count": len(candidates),
        "candidate_ids": [model.id for model, _, _ in candidates],
        "required_tokens": required_tokens,
        "context_usage": context_usage,
        "feedback": {"up": up, "down": down},
        "quality": quality_details,
        "cost": {"selected": costs.get(selected.id), "score": cost_scores.get(selected.id, 0.0)},
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
