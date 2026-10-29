from datetime import UTC, datetime, timedelta

from fastapi import APIRouter, Depends
from sqlalchemy import Float, case, cast, func, select

from mix_agent.api.common import (
    context,
)
from mix_agent.api.schemas import (
    StatisticsView,
    UsageView,
)
from mix_agent.db.models import (
    AutoReliabilityEvent,
    Model,
    ModelUsageEvent,
    PerformanceEvent,
    Provider,
)

router = APIRouter(prefix="/api/v1")


@router.get("/settings/statistics", response_model=StatisticsView)
def settings_statistics(ctx=Depends(context)):
    """Return privacy-minimal, local reliability statistics for the account.

    Aggregation happens in SQL so a busy account never loads a month of raw
    event rows into memory just to render a summary.
    """
    db, s = ctx
    since = datetime.now(UTC) - timedelta(days=30)
    models = {row.id: row.data for row in db.scalars(select(Model).where(Model.owner_id == s.owner_id))}

    def positive_sum(expression):
        value = cast(expression, Float)
        return func.coalesce(func.sum(case((value > 0, value), else_=0)), 0), func.coalesce(
            func.sum(case((value > 0, 1), else_=0)), 0
        )

    auto_model = AutoReliabilityEvent.data["model_id"].astext
    auto_provider = AutoReliabilityEvent.data["provider_id"].astext
    auto_scope = func.coalesce(AutoReliabilityEvent.data["scope"].astext, "chat")
    auto_outcome = AutoReliabilityEvent.data["outcome"].astext
    first_sum, first_count = positive_sum(AutoReliabilityEvent.data["first_output_ms"].astext)
    completion_sum, completion_count = positive_sum(AutoReliabilityEvent.data["completion_ms"].astext)
    auto_rows = db.execute(
        select(
            func.coalesce(auto_model, "unknown"),
            func.coalesce(auto_provider, "unknown"),
            auto_scope,
            func.count(),
            func.coalesce(func.sum(case((auto_outcome == "success", 1), else_=0)), 0),
            first_sum,
            first_count,
            completion_sum,
            completion_count,
        )
        .where(AutoReliabilityEvent.owner_id == s.owner_id, AutoReliabilityEvent.created_at >= since)
        .group_by(auto_model, auto_provider, auto_scope)
    ).all()
    auto_classification = func.coalesce(AutoReliabilityEvent.data["classification"].astext, "other")
    class_rows = db.execute(
        select(
            func.coalesce(auto_model, "unknown"),
            func.coalesce(auto_provider, "unknown"),
            auto_scope,
            auto_classification,
            func.count(),
        )
        .where(
            AutoReliabilityEvent.owner_id == s.owner_id,
            AutoReliabilityEvent.created_at >= since,
            func.coalesce(auto_outcome, "") != "success",
        )
        .group_by(auto_model, auto_provider, auto_scope, auto_classification)
    ).all()
    perf_model = PerformanceEvent.data["model_id"].astext
    perf_provider = PerformanceEvent.data["provider_id"].astext
    perf_scope = func.coalesce(PerformanceEvent.data["mode"].astext, "chat")
    tps_sum, tps_count = positive_sum(PerformanceEvent.data["tokens_per_second"].astext)
    perf_rows = db.execute(
        select(
            func.coalesce(perf_model, "unknown"),
            func.coalesce(perf_provider, "unknown"),
            perf_scope,
            func.count(),
            tps_sum,
            tps_count,
        )
        .where(PerformanceEvent.owner_id == s.owner_id, PerformanceEvent.created_at >= since)
        .group_by(perf_model, perf_provider, perf_scope)
    ).all()

    groups = {}

    def group(model_id, provider_id, scope):
        key = f"{model_id}:{provider_id}:{scope}"
        item = groups.get(key)
        if item is None:
            item = groups[key] = {
                "key": key,
                "model_id": model_id,
                "provider_id": provider_id,
                "scope": scope,
                "model_name": models.get(model_id, {}).get("name")
                or models.get(model_id, {}).get("model_id")
                or model_id,
                "total": 0,
                "success": 0,
                "failure": 0,
                "failure_rate": 0,
                "first_output_ms": None,
                "completion_ms": None,
                "tokens_per_second": None,
                "tps_count": 0,
                "classifications": {},
                "_first_sum": 0.0,
                "_first_count": 0,
                "_completion_sum": 0.0,
                "_completion_count": 0,
                "_tps_sum": 0.0,
                "_tps_count": 0,
            }
        return item

    class_totals = {}
    for model_id, provider_id, scope, classification, count in class_rows:
        item = group(model_id, provider_id, scope)
        item["classifications"][classification] = item["classifications"].get(classification, 0) + count
        class_totals[classification] = class_totals.get(classification, 0) + count
    for (
        model_id,
        provider_id,
        scope,
        count,
        success,
        first_sum,
        first_count,
        completion_sum,
        completion_count,
    ) in auto_rows:
        item = group(model_id, provider_id, scope)
        item["total"] += count
        item["success"] += int(success or 0)
        item["failure"] += count - int(success or 0)
        item["_first_sum"] += float(first_sum or 0)
        item["_first_count"] += int(first_count or 0)
        item["_completion_sum"] += float(completion_sum or 0)
        item["_completion_count"] += int(completion_count or 0)
    total_tps_sum = total_tps_count = 0
    for model_id, provider_id, scope, count, tps_sum, tps_count in perf_rows:
        item = group(model_id, provider_id, scope)
        item["_tps_sum"] += float(tps_sum or 0)
        item["_tps_count"] += int(tps_count or 0)
        total_tps_sum += float(tps_sum or 0)
        total_tps_count += int(tps_count or 0)
    for item in groups.values():
        item["failure_rate"] = round(item["failure"] / item["total"] * 100, 1) if item["total"] else 0
        first_values = item.pop("_first_sum")
        completion_values = item.pop("_completion_sum")
        first_count = item.pop("_first_count")
        completion_count = item.pop("_completion_count")
        item["first_output_ms"] = round(first_values / first_count) if first_count else None
        item["completion_ms"] = round(completion_values / completion_count) if completion_count else None
        item["tps_count"] = item.pop("_tps_count")
        item["tokens_per_second"] = (
            round(item.pop("_tps_sum") / item["tps_count"], 1) if item["tps_count"] else None
        )
    total = {
        "key": "all",
        "model_name": "全体",
        "total": sum(x["total"] for x in groups.values()),
        "success": sum(x["success"] for x in groups.values()),
        "failure": sum(x["failure"] for x in groups.values()),
        "first_output_ms": None,
        "completion_ms": None,
        "tps_count": total_tps_count,
        "tokens_per_second": round(total_tps_sum / total_tps_count, 1) if total_tps_count else None,
        "classifications": class_totals,
    }
    total["failure_rate"] = round(total["failure"] / total["total"] * 100, 1) if total["total"] else 0
    return {
        "retention_days": 30,
        "total": total,
        "groups": sorted(groups.values(), key=lambda x: (-x["total"], x["model_name"])),
    }


@router.get("/settings/usage", response_model=UsageView)
def settings_usage(days: int = 30, ctx=Depends(context)):
    """Return per-model token usage and estimated cost for the account.

    Only token counts and resolved prices are aggregated; prompt and completion
    text are never stored.  Rows whose price is unknown stay in the token totals
    and are reported separately so the cost figure is never silently inflated.
    """
    from mix_agent.usage import RETENTION

    db, s = ctx
    # FastAPI already coerced and validated the query parameter.
    days = max(1, min(RETENTION.days, days))
    since = datetime.now(UTC) - timedelta(days=days)
    models = {row.id: row.data for row in db.scalars(select(Model).where(Model.owner_id == s.owner_id))}
    providers = {
        row.id: row.data for row in db.scalars(select(Provider).where(Provider.owner_id == s.owner_id))
    }

    def token_sum(key):
        return func.coalesce(func.sum(cast(ModelUsageEvent.data[key].astext, Float)), 0)

    requests = func.count()
    input_sum, output_sum = token_sum("input_tokens"), token_sum("output_tokens")
    cached_sum, total_sum = token_sum("cached_tokens"), token_sum("total_tokens")
    cost = ModelUsageEvent.data["cost_usd"].astext
    cost_sum = func.coalesce(func.sum(cast(cost, Float)), 0)
    priced = func.count(cast(cost, Float))
    estimated = func.coalesce(
        func.sum(case((ModelUsageEvent.data["input_source"].astext == "estimated", 1), else_=0)), 0
    )
    model_id = ModelUsageEvent.data["model_id"].astext
    provider_id = ModelUsageEvent.data["provider_id"].astext
    mode = func.coalesce(ModelUsageEvent.data["mode"].astext, "chat")
    source = func.coalesce(ModelUsageEvent.data["pricing_source"].astext, "unknown")
    manual_flag = func.max(case((source == "manual", 1), else_=0))
    priced_source_flag = func.max(case((source != "unknown", 1), else_=0))

    group_rows = db.execute(
        select(
            model_id,
            provider_id,
            mode,
            requests,
            input_sum,
            output_sum,
            cached_sum,
            total_sum,
            cost_sum,
            priced,
            estimated,
            manual_flag,
            priced_source_flag,
        )
        .where(ModelUsageEvent.owner_id == s.owner_id, ModelUsageEvent.created_at >= since)
        .group_by(model_id, provider_id, mode)
    ).all()
    provider_rows = db.execute(
        select(provider_id, requests, input_sum, output_sum, cached_sum, total_sum, cost_sum, priced)
        .where(ModelUsageEvent.owner_id == s.owner_id, ModelUsageEvent.created_at >= since)
        .group_by(provider_id)
    ).all()
    day_rows = db.execute(
        select(
            func.date(ModelUsageEvent.created_at),
            requests,
            input_sum,
            output_sum,
            total_sum,
            cost_sum,
            priced,
        )
        .where(ModelUsageEvent.owner_id == s.owner_id, ModelUsageEvent.created_at >= since)
        .group_by(func.date(ModelUsageEvent.created_at))
        .order_by(func.date(ModelUsageEvent.created_at))
    ).all()

    totals = {
        "requests": 0,
        "input_tokens": 0,
        "output_tokens": 0,
        "cached_tokens": 0,
        "total_tokens": 0,
        "cost_usd": 0.0,
        "priced_requests": 0,
        "unpriced_requests": 0,
        "estimated_requests": 0,
    }
    groups = []
    for (
        model_key,
        provider_key,
        mode_key,
        count,
        input_count,
        output_count,
        cached_count,
        total_count,
        cost_value,
        priced_count,
        estimated_count,
        has_manual,
        has_known,
    ) in group_rows:
        model_key, provider_key = model_key or "unknown", provider_key or "unknown"
        known = bool(priced_count)
        source_name = "manual" if has_manual else ("models_dev" if has_known else "unknown")
        groups.append(
            {
                "key": f"{model_key}:{provider_key}:{mode_key}",
                "model_id": model_key,
                "provider_id": provider_key,
                "mode": mode_key,
                "model_name": models.get(model_key, {}).get("name")
                or models.get(model_key, {}).get("model_id")
                or model_key,
                "provider_name": providers.get(provider_key, {}).get("name"),
                "requests": int(count),
                "input_tokens": int(input_count),
                "output_tokens": int(output_count),
                "cached_tokens": int(cached_count),
                "total_tokens": int(total_count),
                "cost_usd": round(float(cost_value), 6) if known else None,
                "pricing_source": source_name,
                "unpriced": not known,
            }
        )
        totals["requests"] += int(count)
        totals["input_tokens"] += int(input_count)
        totals["output_tokens"] += int(output_count)
        totals["cached_tokens"] += int(cached_count)
        totals["total_tokens"] += int(total_count)
        totals["cost_usd"] += float(cost_value)
        totals["priced_requests"] += int(priced_count)
        totals["estimated_requests"] += int(estimated_count)
    totals["unpriced_requests"] = totals["requests"] - totals["priced_requests"]
    totals["cost_usd"] = round(totals["cost_usd"], 6) if totals["priced_requests"] else None
    provider_view = []
    for (
        provider_key,
        count,
        input_count,
        output_count,
        cached_count,
        total_count,
        cost_value,
        priced_count,
    ) in provider_rows:
        provider_key = provider_key or "unknown"
        provider_view.append(
            {
                "provider_id": provider_key,
                "provider_name": providers.get(provider_key, {}).get("name"),
                "requests": int(count),
                "input_tokens": int(input_count),
                "output_tokens": int(output_count),
                "cached_tokens": int(cached_count),
                "total_tokens": int(total_count),
                "cost_usd": round(float(cost_value), 6) if priced_count else None,
                "unpriced": not bool(priced_count),
            }
        )
    series = [
        {
            "date": day.isoformat() if hasattr(day, "isoformat") else str(day),
            "requests": int(count),
            "input_tokens": int(input_count),
            "output_tokens": int(output_count),
            "total_tokens": int(total_count),
            "cost_usd": round(float(cost_value), 6) if priced_count else None,
        }
        for day, count, input_count, output_count, total_count, cost_value, priced_count in day_rows
    ]

    return {
        "retention_days": RETENTION.days,
        "total": totals,
        "days": series,
        "groups": sorted(groups, key=lambda x: (-x["total_tokens"], x["model_name"])),
        "providers": sorted(provider_view, key=lambda x: -x["total_tokens"]),
    }
