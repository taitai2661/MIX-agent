"""Server-enforced policies for the three user-facing response modes."""

from copy import deepcopy

MODE_POLICIES = {
    "chat": {"label": "通常", "max_seconds": 1200, "max_steps": 12, "max_tool_calls": 12,
             "planning": False, "delegation": False, "checkpointing": False,
             "background_processes": False, "persistent_browser": False, "auto_skill_learning": False},
    "thinking": {"label": "深く考える", "max_seconds": 2700, "max_steps": 24, "max_tool_calls": 24,
                  "planning": False, "delegation": False, "checkpointing": False,
                  "background_processes": False, "persistent_browser": False, "auto_skill_learning": False},
    "agent": {"label": "長作業", "max_seconds": 5400, "max_steps": 300, "max_tool_calls": 750,
              "planning": True, "delegation": True, "checkpointing": True,
              "background_processes": True, "persistent_browser": True, "auto_skill_learning": True},
}

MODE_PROMPTS = {
    "chat": (
        "\n## Response mode: Normal\n"
        "Respond naturally to the user's immediate request and conversation context. Match the "
        "detail to the question: be brief for simple questions and explain or investigate when "
        "the task needs it. Ask a clarifying question only when the missing answer matters; "
        "otherwise use available context and take useful small actions in this Run. Continue "
        "after a tool result or failure until you can give a meaningful answer. Do not delegate "
        "or start background work. Never switch modes yourself."
    ),
    "thinking": (
        "\n## Response mode: Think deeply\n"
        "Reason carefully before answering: identify the decision or question, inspect assumptions, "
        "constraints, counterexamples, risks, and plausible alternatives. Gather and cross-check "
        "evidence when it would materially change the conclusion. Give the conclusion first, followed "
        "by a concise explanation of the decisive evidence, trade-offs, and any remaining uncertainty; "
        "do not expose private chain-of-thought. Do not create a long-running plan, delegate, persist "
        "background work, or switch modes. If execution needs sustained iteration, recommend Long work "
        "mode and say why."
    ),
    "agent": (
        "\n## Response mode: Long work\n"
        "Own the requested outcome within granted permissions. First inspect the current state and "
        "define observable success. Publish and maintain a concise checklist with update_plan, then "
        "iterate through implementation, observation, validation, and correction. Take the next safe "
        "action instead of stopping at a plan or progress promise. Preserve user data, public contracts, "
        "and unrelated work; make no destructive or external change without the required authorization. "
        "Track remaining work with update_plan.pending and clear it only after the work is done. "
        "After failures, diagnose from evidence and adapt. Check the requested outcome against "
        "actual tool results, and record verification with update_plan after the last change. "
        "Continue until the outcome is verified or a real authorization, user-choice, or budget "
        "blocker remains. Never claim completion for unverified work. Finish with the result, "
        "verification evidence, and only meaningful follow-up work."
    ),
}


def mode_policy(mode: str) -> dict:
    return deepcopy(MODE_POLICIES.get(mode, MODE_POLICIES["chat"]))


def apply_mode_defaults(snapshot: dict, mode: str) -> dict:
    """Freeze policy into a Run; Agent presets can still narrow its budget."""
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
