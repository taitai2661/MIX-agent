"""ContextBuilder: single funnel for all run triggers.

Builds provider input as::

    System / Task State / Prior Summary / Relevant memory (role-organized)
    / Skills / Knowledge / Recent Conversation / Current User Message

and enforces model-aware budgets *before* sending. Legacy
``Run.data.history`` payloads are ingested read-only for back-compat.

The "Relevant memory" block is now rendered in role buckets so the model can
distinguish Decision / Failure / Experience / Fact from one glance instead of
digesting an undifferentiated list of "traces".
"""

from __future__ import annotations

import json

from mix_agent.context import budget as budget_mod
from mix_agent.context import head_blocks, retrievers, tokens, tools_selector
from mix_agent.context import task_state as task_state_mod
from mix_agent.context.references import (
    extract_data_url_images,
    resolve_message_images,
)
from mix_agent.context.summary import render as render_summary
from mix_agent.context.types import CONTEXT_VERSION, ContextBudgetError
from mix_agent.memory import runtime as mem_runtime
from mix_agent.memory import types as mem_types

TOOL_PROTOCOL_RESERVED = 2000


def _message_text(message: dict) -> str:
    return str(message.get("content") or "")


def _normalize_legacy(history: list[dict]) -> list[dict]:
    """Coerce old Run.data.history entries to the canonical message shape."""
    normalized = []
    for item in history or []:
        if not isinstance(item, dict):
            continue
        role = item.get("role", "user")
        normalized.append(
            {
                "role": role,
                "content": _message_text(item),
                "images": list(item.get("images") or []),
                "image_refs": list(item.get("image_refs") or []),
                "name": item.get("name"),
                "call_id": item.get("call_id"),
                "tool_calls": list(item.get("tool_calls") or []),
                "tool_ref": item.get("tool_ref"),
            }
        )
    return normalized


def strip_inline_images(messages: list[dict]) -> tuple[list[dict], list[dict]]:
    """Hoist base64 images out of messages; returns (messages, extracted)."""
    cleaned, extracted_all = [], []
    for message in messages or []:
        updated, extracted = extract_data_url_images(message)
        cleaned.append(updated)
        extracted_all.extend(extracted)
    return cleaned, extracted_all


def select_recent(messages: list[dict], budget: int, model_id: str = "") -> tuple[list[dict], list[dict]]:
    """Sliding window from the newest message while budget allows.

    Always keeps the latest user message. Excluded (older) messages are
    returned for progressive summarization instead of being dropped silently.

    Tool-call/result pairing is preserved: an ``assistant`` message carrying
    ``tool_calls`` and its matching ``role: "tool"`` results are treated as a
    unit. Slicing between them produces a protocol violation (the provider
    rejects an orphan ``function_call_output``/``tool_result``), so the window
    breaks at pair boundaries.
    """
    if not messages:
        return [], []
    # Latest user message (or latest message) is mandatory.
    mandatory_idx = max(
        (i for i, m in enumerate(messages) if m.get("role") == "user"),
        default=len(messages) - 1,
    )
    included_rev, excluded = [], []
    used = 0
    for idx in range(len(messages) - 1, -1, -1):
        message = messages[idx]
        cost = tokens.count(_message_text(message), model_id) + 8
        cost += sum(len(str(image)) // 3 for image in message.get("images") or [])
        cost += len(message.get("image_refs") or []) * 768
        cost += sum(len(str(call.get("arguments") or "")) // 2 + 40 for call in message.get("tool_calls") or [])
        if idx == mandatory_idx:
            included_rev.append(message)
            used += cost
            continue
        if used + cost <= max(0, budget):
            included_rev.append(message)
            used += cost
        else:
            excluded.append(message)
    included = list(reversed(included_rev))
    excluded = list(reversed(excluded))
    # Mandatory message must survive even when everything else is evicted.
    if mandatory_idx is not None and not any(m is messages[mandatory_idx] for m in included):
        included.append(messages[mandatory_idx])
    included, excluded = _repair_tool_pairing(included, excluded)
    return included, excluded


def _repair_tool_pairing(included: list[dict], excluded: list[dict]) -> tuple[list[dict], list[dict]]:
    """Drop orphan tool calls/results so the provider never sees a broken pair.

    An assistant message may carry ``tool_calls`` whose ``role: "tool"`` results
    fell on the other side of the budget window (or vice versa). Keeping only
    one side is a protocol violation, so we strip the dangling half. Results are
    dropped preferentially: losing an already-computed result is cheaper than
    losing the assistant turn that frames the next action.
    """
    included_ids = {m.get("call_id") for m in included if m.get("role") == "tool"}
    included_tool_calls = set()
    for message in included:
        for call in message.get("tool_calls") or []:
            call_id = call.get("id")
            if isinstance(call_id, str) and call_id:
                included_tool_calls.add(call_id)
    # 1. Tool results whose assistant turn was evicted → move to excluded.
    new_included: list[dict] = []
    orphaned_results: list[dict] = []
    for message in included:
        if message.get("role") == "tool" and message.get("call_id") not in included_tool_calls:
            orphaned_results.append(message)
        else:
            new_included.append(message)
    included = new_included
    # 2. Assistant turns with tool_calls whose results were evicted → strip the
    #    tool_calls so the message is a plain assistant turn.
    final: list[dict] = []
    for message in included:
        tool_calls = message.get("tool_calls") or []
        if tool_calls and message.get("role") == "assistant":
            kept = [c for c in tool_calls if c.get("id") in included_ids]
            if len(kept) != len(tool_calls):
                message = {**message, "tool_calls": kept or None}
        final.append(message)
    included = final
    if orphaned_results:
        excluded = orphaned_results + excluded
    return included, excluded


def compress_overflow(
    recent: list[dict],
    excluded: list[dict],
    budgets: dict,
    model_id: str = "",
) -> tuple[list[dict], list[dict]]:
    """Overflow order: tool raw already ref'd; move old conversation to summary queue."""
    _ = (budgets, model_id)
    return recent, excluded


def build_initial(
    *,
    system_text: str,
    prior_messages: list[dict],
    current_message: dict,
    task_goal: str = "",
    memories: list[dict] | None = None,
    skills: list[dict] | None = None,
    knowledge: list[dict] | None = None,
    tools: list[dict] | None = None,
    window_info: dict,
    model_id: str = "",
    trigger: str = "interactive",
    previous_summary: str = "",
    task_state_value: dict | None = None,
) -> dict:
    """Assemble initial context from categorized parts within budget."""
    from mix_agent.context.types import TRIGGER_TYPES

    if trigger not in TRIGGER_TYPES:
        trigger = "interactive"
    tool_cost = tools_selector.schema_cost(tools or [], model_id)
    # Tool schemas share the input budget on providers that inline them.
    total = budget_mod.input_budget(window_info, tool_schema_tokens=tool_cost)
    budgets = budget_mod.category_budgets(total)

    system_text = system_text or ""
    task_state_rendered = task_state_mod.render(task_state_mod.ensure(task_state_value, task_goal))
    summary_rendered = render_summary(previous_summary)

    # Memory rendering is now role-organized: each role group is fitted
    # against a slice of the memory budget so the most important buckets
    # (decisions, failures) keep priority over generic facts.
    memory_included, memory_excluded, memory_block, memory_groups = _render_memory(
        memories or [], budgets["memory"], model_id,
    )
    skills_included, skills_excluded = _fit_block(skills or [], budgets["skills"], model_id)
    knowledge_included, knowledge_excluded = _fit_block(knowledge or [], budgets["knowledge"], model_id)

    skills_block = retrievers.render_block(head_blocks.SKILLS, skills_included)
    knowledge_block = retrievers.render_block(head_blocks.KNOWLEDGE, knowledge_included)

    head_parts = [system_text]
    if task_state_rendered:
        head_parts.append(task_state_rendered)
    if summary_rendered:
        head_parts.append(summary_rendered)
    if memory_block:
        head_parts.append(memory_block)
    if skills_block:
        head_parts.append(skills_block)
    if knowledge_block:
        head_parts.append(knowledge_block)
    head_text = "\n".join(part for part in head_parts if part)
    head_tokens = tokens.count(head_text, model_id)

    conversation = [*_normalize_legacy(prior_messages), _normalize_legacy([current_message])[0]]
    conversation, _extracted = strip_inline_images(conversation)
    # Reserve head cost from the recent-conversation share first; reflow handles the rest.
    recent_budget = max(1000, budgets["recent_conversation"] + budgets.get("reserve", 0) - head_tokens)
    # Prior messages exclude the just-added current message for eviction purposes.
    recent, evicted = select_recent(conversation, recent_budget, model_id)
    recent, evicted = compress_overflow(recent, evicted, budgets, model_id)

    provider_messages = [{"role": "system", "content": head_text}]
    for message in recent:
        entry: dict = {"role": message["role"], "content": message["content"]}
        if message.get("images"):
            entry["images"] = message["images"]
        if message.get("image_refs"):
            entry["image_refs"] = message["image_refs"]
        if message.get("name"):
            entry["name"] = message["name"]
        if message.get("call_id"):
            entry["call_id"] = message["call_id"]
        if message.get("tool_calls"):
            entry["tool_calls"] = message["tool_calls"]
        if message.get("tool_ref"):
            entry["tool_ref"] = message["tool_ref"]
        provider_messages.append(entry)

    # NOTE: total already excludes tool_schema_tokens, so estimated here must be
    # messages-only (no double counting).
    estimated = tokens.count_messages(provider_messages, model_id)
    if estimated > total + TOOL_PROTOCOL_RESERVED and len(recent) <= 1:
        raise ContextBudgetError(
            f"context budget exceeded: estimated {estimated} > {total} tokens "
            f"(window {window_info.get('context_window')})"
        )

    trace = {
        "model": model_id,
        "trigger": trigger,
        "context_version": CONTEXT_VERSION,
        "context_window": window_info.get("context_window"),
        "input_budget": total,
        "estimated_input_tokens": estimated,
        "tool_schema_tokens": tool_cost,
        "categories": {
            "system": tokens.count(system_text, model_id),
            "task_state": tokens.count(task_state_rendered, model_id),
            "summary": tokens.count(summary_rendered, model_id),
            "recent_conversation": tokens.count_messages(
                [m for m in provider_messages if m.get("role") != "system"], model_id
            ),
            "memory": tokens.count(memory_block, model_id),
            "skills": tokens.count(skills_block, model_id),
            "knowledge": tokens.count(knowledge_block, model_id),
            "tools": tool_cost,
        },
        "included": {
            "recent_count": len(recent),
            "memory_ids": [m.get("id") for m in memory_included],
            "memory_groups": memory_groups,
            "skill_ids": [m.get("id") for m in skills_included],
            "knowledge_ids": [m.get("id") or m.get("chunk_id") for m in knowledge_included],
        },
        "excluded": {
            "evicted_count": len(evicted),
            "memory": memory_excluded,
            "skills": skills_excluded,
            "knowledge": knowledge_excluded,
        },
        "summarized": [],
        "retrieved": [],
    }
    return {
        "messages": provider_messages,
        "recent": recent,
        "evicted": evicted,
        "trace": trace,
        "task_state": task_state_mod.ensure(task_state_value, task_goal),
        "budgets": budgets,
        "input_budget": total,
    }


def resolve_for_provider(messages: list[dict], image_loader) -> list[dict]:
    """Resolve image_refs at send time; missing files degrade, never crash."""
    resolved = []
    missing_all: list = []
    for message in messages or []:
        updated = resolve_message_images(message, image_loader)
        missing_all.extend(updated.pop("missing_image_refs", []))
        cleaned = {k: v for k, v in updated.items() if k not in ("image_refs", "tool_ref")}
        resolved.append(cleaned)
    # Missing refs are reported via trace by the caller; keep transport clean.
    _ = missing_all
    return resolved


def _fit_block(items: list[dict], budget: int, model_id: str = ""):
    return retrievers.fit_items(items, budget, model_id)


# Priority shares inside the memory budget.  Decisions and Failures are
# surfaced even when the budget is tight; Questions/Experiences share the
# remainder so we don't drown the model in low-confidence memories.
_MEMORY_ROLE_PRIORITY: tuple[tuple[str, float], ...] = (
    (mem_types.ROLE_DECISION, 0.28),
    (mem_types.ROLE_FAILURE, 0.22),
    (mem_types.ROLE_PREFERENCE, 0.12),
    (mem_types.ROLE_GOAL, 0.10),
    (mem_types.ROLE_FACT, 0.18),
    (mem_types.ROLE_EXPERIENCE, 0.06),
    (mem_types.ROLE_HYPOTHESIS, 0.02),
    (mem_types.ROLE_QUESTION, 0.02),
)


def _render_memory(
    memories: list[dict],
    budget: int,
    model_id: str = "",
) -> tuple[list[dict], list[dict], str, dict[str, int]]:
    """Render the memory block in role-organized buckets.

    The output keeps the highest-priority roles (decisions, failures) visible
    even when the total budget is small, and surfaces only the most relevant
    Facts / Experiences when there is room.  Returns ``(included, excluded,
    block_text, group_counts)``.
    """
    if not memories or budget <= 0:
        return [], [{"id": m.get("id"), "reason": "no_budget"} for m in memories], "", {}
    grouped = mem_runtime.group_for_context(memories)
    grouped_sets: dict[str, set[str]] = {key: {item["id"] for item in items if item.get("id")} for key, items in grouped.items()}
    flat_ids: dict[str, dict] = {item["id"]: item for item in memories if item.get("id")}
    included: list[dict] = []
    excluded: list[dict] = []
    sections: list[str] = []
    # Report every bucket, including the empty ones: the trace is rendered as a
    # per-bucket panel, so a missing key would be indistinguishable from a bug.
    counts: dict[str, int] = {key: 0 for key in grouped}
    used = 0
    label_map = {
        "decisions": "Relevant active Decisions",
        "failures": "Past failures to avoid",
        "preferences": "User preferences",
        "goals": "Active goals / constraints",
        "facts": "Relevant facts",
        "experiences": "Relevant experiences",
        "questions": "Open hypotheses / questions",
        "other": "Other memory",
    }
    for role, share in _MEMORY_ROLE_PRIORITY:
        bucket_key = _bucket_key_for_role(role)
        bucket = grouped.get(bucket_key) or []
        if not bucket:
            continue
        bucket_budget = max(120, int(budget * share))
        bucket_included, bucket_excluded = _fit_block(bucket, bucket_budget, model_id)
        counts[bucket_key] = len(bucket_included)
        included.extend(bucket_included)
        for entry in bucket_excluded:
            excluded.append({"id": entry.get("id"), "reason": f"budget:{bucket_key}"})
        if bucket_included:
            text = json.dumps(bucket_included, ensure_ascii=False)
            if len(text) > 1200:
                text = text[:1200] + "…(truncated)"
            sections.append(f"{label_map[bucket_key]}:\n{text}")
        used += sum(len(json.dumps(m, ensure_ascii=False)) for m in bucket_included) // 2
    # Leftover budget lets the highest-priority buckets absorb spillover.
    leftover = max(0, budget - used)
    if leftover > 200:
        for role, share in _MEMORY_ROLE_PRIORITY:
            bucket_key = _bucket_key_for_role(role)
            bucket = grouped.get(bucket_key) or []
            room_ids = grouped_sets.get(bucket_key, set()) - {m["id"] for m in included}
            extras = [flat_ids[i] for i in room_ids if i in flat_ids]
            if not extras:
                continue
            extra_included, _ = _fit_block(extras, leftover, model_id)
            for item in extra_included:
                if item["id"] in {m["id"] for m in included}:
                    continue
                included.append(item)
                counts[bucket_key] = counts.get(bucket_key, 0) + 1
                leftover -= len(json.dumps(item, ensure_ascii=False)) // 2 + 20
                if leftover <= 100:
                    break
            if leftover <= 100:
                break
    block = head_blocks.heading(head_blocks.MEMORY) + "\n" + "\n\n".join(sections) if sections else ""
    return included, excluded, block, counts


def _bucket_key_for_role(role: str) -> str:
    return {
        mem_types.ROLE_DECISION: "decisions",
        mem_types.ROLE_FAILURE: "failures",
        mem_types.ROLE_PREFERENCE: "preferences",
        mem_types.ROLE_GOAL: "goals",
        mem_types.ROLE_CONSTRAINT: "goals",
        mem_types.ROLE_FACT: "facts",
        mem_types.ROLE_EXPERIENCE: "experiences",
        mem_types.ROLE_SOLUTION: "experiences",
        mem_types.ROLE_OBSERVATION: "experiences",
        mem_types.ROLE_HYPOTHESIS: "questions",
        mem_types.ROLE_QUESTION: "questions",
    }.get(role, "other")


def history_from_built(built: dict) -> list[dict]:
    """Persistable history: provider messages incl. lightweight ref pointers."""
    return json.loads(json.dumps(built.get("messages") or []))
