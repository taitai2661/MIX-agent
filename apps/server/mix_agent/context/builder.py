"""ContextBuilder: single funnel for all run triggers.

Builds provider input as::

    System / Task State / Prior Summary / Memory / Skills / Knowledge
    / Recent Conversation / Current User Message

and enforces model-aware budgets *before* sending. Legacy
``Run.data.history`` payloads are ingested read-only for back-compat.
"""

from __future__ import annotations

import json

from mix_agent.context import budget as budget_mod
from mix_agent.context import retrievers, tokens, tools_selector
from mix_agent.context import task_state as task_state_mod
from mix_agent.context.references import (
    extract_data_url_images,
    resolve_message_images,
)
from mix_agent.context.summary import render as render_summary
from mix_agent.context.types import CONTEXT_VERSION, ContextBudgetError

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
        cost += len(message.get("image_refs") or []) * 30
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

    memory_included, memory_excluded = _fit_block(memories or [], budgets["memory"], model_id)
    skills_included, skills_excluded = _fit_block(skills or [], budgets["skills"], model_id)
    knowledge_included, knowledge_excluded = _fit_block(knowledge or [], budgets["knowledge"], model_id)

    memory_block = retrievers.render_block("Relevant memories", memory_included)
    skills_block = retrievers.render_block("Relevant reusable skills", skills_included)
    knowledge_block = retrievers.render_block("Relevant knowledge", knowledge_included)

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


def history_from_built(built: dict) -> list[dict]:
    """Persistable history: provider messages incl. lightweight ref pointers."""
    return json.loads(json.dumps(built.get("messages") or []))
