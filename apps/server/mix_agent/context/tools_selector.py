"""Tool schema cost awareness. Full lazy loading arrives in Phase 3."""

from __future__ import annotations

from mix_agent.context import tokens


def schema_cost(tools: list[dict], model_id: str = "") -> int:
    """Token estimate for tool definitions sent alongside messages."""
    return tokens.count_tool_schemas(tools or [], model_id)


def select_all(tools: list[dict]) -> tuple[list[dict], list[dict]]:
    """Phase 1 passthrough: send every allowed schema, but report its cost.

    Phase 3 will replace this with category/server routing. The return shape
    (selected, deferred) is already the lazy-loading contract.
    """
    return list(tools or []), []
