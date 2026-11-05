"""Tool selection for a new message run."""

from mix_agent.api.routes_conversations import effective_tool_ids
from mix_agent.tools.registry import BUILTINS


def resolve_tool_ids(agent, requested_agent_id, mode, temporary_mode, body, settings):
    """Resolve the effective tool id list for a run given mode, agent and settings."""
    tool_ids = agent.get("tool_ids", [t["id"] for t in BUILTINS])
    if mode == "agent" and not temporary_mode and not requested_agent_id and agent.get("auto_learn", True):
        tool_ids = list(
            dict.fromkeys(
                [
                    *tool_ids,
                    "memory_search",
                    "memory_add",
                    "memory_update",
                    "skill_search",
                    "skill_add",
                    "skill_update",
                ]
            )
        )
    tool_ids = (
        effective_tool_ids(settings, agent, tool_ids) if (not temporary_mode or body.allow_tools) else []
    )
    return [
        tool_id for tool_id in tool_ids if not temporary_mode or not tool_id.startswith(("memory_", "skill_"))
    ]
