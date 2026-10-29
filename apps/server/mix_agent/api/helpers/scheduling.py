"""Scheduling helpers for creating engine Runs from scheduled jobs.

These functions were extracted from mix_agent.api.routes to keep the route
module manageable.  They are re-exported from routes.py for backward compat.
"""

from fastapi import HTTPException
from sqlalchemy import select

from mix_agent.context import builder as context_builder
from mix_agent.context import retrievers as context_retrievers
from mix_agent.context import task_state as context_task_state
from mix_agent.db.models import (
    Agent,
    Conversation,
    Message,
    Model,
    Provider,
    Run,
    Settings,
    now,
)
from mix_agent.providers.reasoning import resolve_reasoning
from mix_agent.routing import effective_capabilities, select_auto_model
from mix_agent.runs.mode_policy import apply_mode_defaults, mode_prompt, tool_allowed
from mix_agent.tools.registry import registry


def enqueue_scheduled_run(db, owner_id, job, scheduled):
    """Create a normal engine Run from a scheduled job using ContextBuilder."""
    # Lazy imports to avoid circular dependency with routes.py
    from mix_agent.api.routes import (
        _model_window_info,
        effective_tool_ids,
        own,
    )
    from mix_agent.providers.probing import tool_capability

    data = job.data
    if data["target_type"] == "conversation":
        conversation = own(db, Conversation, data["target_id"], owner_id)
    else:
        conversation = db.scalar(
            select(Conversation).where(
                Conversation.owner_id == owner_id,
                Conversation.data["scheduled_job_id"].as_string() == job.id,
            )
        )
        if not conversation:
            conversation = Conversation(
                owner_id=owner_id,
                data={
                    "title": data["name"],
                    "scheduled_job_id": job.id,
                    "cron_hidden": True,
                },
            )
            db.add(conversation)
            db.flush()
    settings = own(db, Settings, "settings", owner_id)
    agent = own(db, Agent, data["target_id"], owner_id).data if data["target_type"] == "agent" else {}
    selection = conversation.data.get("selection", {}) if data["target_type"] == "conversation" else {}
    requested_model_id = (
        agent.get("model_id") or selection.get("model_id") or settings.data.get("default_model_id") or "auto"
    )
    mode = agent.get("mode") or selection.get("mode") or "agent"
    if requested_model_id == "auto":
        allowed = settings.data.get("auto_model_ids", [])
        task_content = data["prompt"] + ("\n" + agent.get("system_prompt", "") if mode == "agent" else "")
        model, auto_selection = select_auto_model(
            db,
            owner_id,
            allowed,
            task_content,
            mode,
            [],
            bool(agent.get("tool_ids")),
            [data["prompt"]],
            agent.get("model_settings", {}).get("max_output_tokens", 4096),
            scheduled.id,
            0,
            priority=settings.data.get("auto_priority", "balanced"),
        )
        if not model:
            raise HTTPException(422, (auto_selection or {}).get("reason", "Autoモデルを選択できません"))
    else:
        model, auto_selection = own(db, Model, requested_model_id, owner_id), None
    provider = own(db, Provider, model.data["provider_id"], owner_id)
    tool_ids = effective_tool_ids(settings, agent, agent.get("tool_ids", []))
    # Scheduled runs are unattended, so capability confirmation is left to the
    # scheduler tick (probe_due_models).  An unverified model keeps its tools
    # disabled here, matching the historical behaviour and avoiding added
    # latency on every cron tick.
    if tool_ids and tool_capability(model.data) is not True:
        tool_ids = []
    try:
        reasoning = resolve_reasoning(
            provider.data["kind"],
            model.data["model_id"],
            effective_capabilities(model.data),
            mode,
            agent.get("model_settings", {}),
        )
    except ValueError:
        reasoning = {"policy": "off", "request": {}, "summary": False}
    # Same pipeline as interactive: build snapshot/history through ContextBuilder
    # so scheduled runs get memory/skills/budget handling instead of a fork.
    # (Scheduled prompts carry no attachments, so this stays synchronous.)
    prior_messages = list(
        db.scalars(
            select(Message).where(Message.conversation_id == conversation.id).order_by(Message.created_at)
        )
    )
    available = registry(db, owner_id)
    tool_ids = [t for t in tool_ids if t in available and tool_allowed(mode, available[t])]
    snapshot = apply_mode_defaults(
        {
            **agent,
            "agent_id": data["target_id"]
            if data["target_type"] == "agent"
            else selection.get("agent_id", ""),
            "mode": mode,
            "model_id": model.data["model_id"],
            "model_record_id": model.id,
            "requested_model_id": requested_model_id,
            "auto_retry_count": 0,
            "auto_dynamic_switching": bool(settings.data.get("auto_dynamic_switching", True))
            if requested_model_id == "auto"
            else False,
            "auto_priority": settings.data.get("auto_priority", "balanced")
            if requested_model_id == "auto"
            else "balanced",
            "provider": provider.data,
            "provider_record_id": provider.id,
            "tool_ids": tool_ids,
            "tools": [available[t] for t in tool_ids if t in available],
            "reasoning": reasoning,
        },
        mode,
    )
    system_text = agent.get(
        "system_prompt", "You are MIX, a helpful assistant. Respond in the user's language."
    )
    snapshot["mode_prompt"] = mode_prompt(mode)
    system_text += snapshot["mode_prompt"]
    system_text += "\nThis is an unattended scheduled run. Use only already-allowed tools; never request approval. Give a concise user-facing result."
    memory_settings = {"result_limit": settings.data.get("memory_result_limit", 8)}
    memories = context_retrievers.search_memories(
        db, owner_id, data["prompt"], agent.get("memory_scopes", ["user"]), memory_settings
    )
    skill_rows = context_retrievers.search_skills(
        db, owner_id, data["prompt"], agent.get("skill_ids") or None
    )
    knowledge_rows = context_retrievers.search_knowledge(db, owner_id, data["prompt"])
    window_info = _model_window_info(model, snapshot)
    built = context_builder.build_initial(
        system_text=system_text,
        prior_messages=[
            {"role": m.data.get("role", "user"), "content": m.data.get("content", "")} for m in prior_messages
        ],
        current_message={"role": "user", "content": data["prompt"]},
        task_goal=data["prompt"],
        memories=memories,
        skills=skill_rows,
        knowledge=knowledge_rows,
        tools=[available[t] for t in tool_ids if t in available],
        window_info=window_info,
        model_id=model.data.get("model_id", ""),
        trigger="scheduled",
        previous_summary="",
        task_state_value=context_task_state.blank(),
    )
    history = context_builder.history_from_built(built)
    snapshot["context_window_info"] = window_info
    snapshot["_context_bootstrap"] = {
        "task_state": built["task_state"],
        "summary": "",
        "budgets": built["budgets"],
        "input_budget": built["input_budget"],
    }
    attempt = int(scheduled.data.get("attempt", 0))
    auto_routing = (
        {
            "allowed_ids": settings.data.get("auto_model_ids", []),
            "content": data["prompt"] + ("\n" + agent.get("system_prompt", "") if mode == "agent" else ""),
            "mode": mode,
            "artifact_mimes": [],
            "tools_required": bool(tool_ids),
            "reserved_output_tokens": agent.get("model_settings", {}).get("max_output_tokens", 4096),
            "attachment_bytes": 0,
            "dynamic_switching": bool(settings.data.get("auto_dynamic_switching", True)),
            "priority": settings.data.get("auto_priority", "balanced"),
        }
        if requested_model_id == "auto"
        else None
    )
    run = Run(
        owner_id=owner_id,
        conversation_id=conversation.id,
        request_key=f"schedule-{scheduled.id}-{attempt}",
        data={
            "snapshot": snapshot,
            "history": history,
            "steps": 0,
            "tool_count": 0,
            "scheduled_run_id": scheduled.id,
            "requested_model_id": requested_model_id,
            "auto_selection": auto_selection,
            "auto_routing": auto_routing,
            "auto_model_history": (
                [{"step": 1, "to_model_record_id": model.id, "model_id": model.data["model_id"]}]
                if auto_selection
                else []
            ),
            "task_state": built["task_state"],
            "summary": {"text": "", "covered_count": 0, "updated_at": None},
            "context_trace": built["trace"],
            "context_version": 1,
            "trigger_type": "scheduled",
            "memory_trace_ids": [m.get("id") for m in memories if isinstance(m, dict) and m.get("id")],
            "image_refs": [],
            "tool_refs": [],
        },
    )
    db.add(run)
    db.add(
        Message(
            owner_id=owner_id,
            conversation_id=conversation.id,
            data={"role": "user", "content": data["prompt"], "scheduled_run_id": scheduled.id},
        )
    )
    db.flush()
    scheduled.run_id = run.id
    scheduled.status = "running"
    scheduled.data = {**scheduled.data, "snapshot": snapshot}
    conversation.data = {**conversation.data, "last_message_at": now().isoformat()}
    return run
