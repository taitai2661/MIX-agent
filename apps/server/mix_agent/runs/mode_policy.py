"""Server-enforced policies for the three user-facing response modes."""

from copy import deepcopy

MODE_POLICIES = {
    "chat": {"label": "通常", "max_seconds": 900, "max_steps": 8, "max_tool_calls": 8,
             "planning": False, "delegation": False, "checkpointing": False,
             "background_processes": False, "persistent_browser": False, "auto_skill_learning": False},
    "thinking": {"label": "深く考える", "max_seconds": 1800, "max_steps": 16, "max_tool_calls": 16,
                  "planning": False, "delegation": False, "checkpointing": False,
                  "background_processes": False, "persistent_browser": False, "auto_skill_learning": False},
    "agent": {"label": "長作業", "max_seconds": 3600, "max_steps": 200, "max_tool_calls": 500,
              "planning": True, "delegation": True, "checkpointing": True,
              "background_processes": True, "persistent_browser": True, "auto_skill_learning": True},
}

MODE_PROMPTS = {
    "chat": (
        "\nResponse mode: Normal. Give a short, direct, proportionate answer. Use only the "
        "few tools that materially help this small task. Do not create a plan, delegate work, "
        "start background work, or turn it into a broad investigation. Recommend Long work "
        "mode for sustained work; never switch modes yourself."
    ),
    "thinking": (
        "\nResponse mode: Think deeply. Examine assumptions, constraints, counterexamples, risks, "
        "and plausible alternatives. Gather and cross-check evidence when useful, then give a clear "
        "conclusion without exposing private chain-of-thought. Do not create a long-running plan, "
        "delegate, persist background work, or switch modes. Recommend Long work mode when needed."
    ),
    "agent": (
        "\nResponse mode: Long work. Own the requested outcome within granted permissions. Define "
        "success, publish and maintain a concise checklist with update_plan, then iterate through "
        "plan, execution, observation, validation, and correction. Inspect current state first, "
        "preserve data and unrelated work, validate intermediate results, adapt after failures, and continue until completion or "
        "a real authorization or user-choice blocker. Verify the result before finishing."
    ),
}


def mode_policy(mode: str) -> dict:
    return deepcopy(MODE_POLICIES.get(mode, MODE_POLICIES["chat"]))


def apply_mode_defaults(snapshot: dict, mode: str) -> dict:
    """Freeze policy into a Run; only Long work accepts legacy Agent budget overrides."""
    policy = mode_policy(mode)
    if mode == "agent":
        for key in ("max_seconds", "max_steps", "max_tool_calls"):
            if isinstance(snapshot.get(key), int):
                policy[key] = snapshot[key]
    limits = {key: policy[key] for key in ("max_seconds", "max_steps", "max_tool_calls")}
    return {**snapshot, **limits, "policy": policy}


def mode_prompt(mode: str) -> str:
    return MODE_PROMPTS.get(mode, MODE_PROMPTS["chat"])


def tool_allowed(mode: str, tool: dict, arguments: dict | None = None) -> bool:
    """Apply mode boundaries before user permission rules."""
    if mode not in tool.get("allowed_modes", ("chat", "thinking", "agent")):
        return False
    if tool.get("id") == "run_terminal" and (arguments or {}).get("background"):
        return mode_policy(mode)["background_processes"]
    return True
