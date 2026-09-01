"""Stable execution policies for the three user-facing response modes."""

MODE_DEFAULTS = {
    "chat": {"max_seconds": 900, "max_tool_calls": 8},
    "thinking": {"max_seconds": 1800, "max_tool_calls": 16},
    "agent": {"max_seconds": 3600, "max_tool_calls": 500},
}

MODE_PROMPTS = {
    "chat": (
        "\nResponse mode: Normal. Give a direct, proportionate answer. Decide whether "
        "tools or extra investigation materially improve correctness, and use them only "
        "when useful. Complete small tasks without unnecessary planning overhead."
    ),
    "thinking": (
        "\nResponse mode: Think deeply. Before answering, examine assumptions, constraints, "
        "edge cases, and plausible alternatives. For factual or technical work, gather and "
        "cross-check relevant evidence when tools are available. Resolve contradictions, "
        "verify important conclusions, and then give a clear answer rather than exposing "
        "private chain-of-thought. Do not stop at a shallow first answer."
    ),
    "agent": (
        "\nResponse mode: Long-running autonomous work. Own the user's requested outcome "
        "within the granted permissions. Inspect the current state before changing it; create "
        "and maintain a concise task checklist with update_plan for multi-step work; execute "
        "the steps, validate intermediate results, and adapt after failures instead of stopping "
        "at the first obstacle. Preserve existing data and unrelated changes. Continue until "
        "the requested outcome is completed, a real authorization or user-choice blocker is "
        "reached, or the execution budget is exhausted. Do not merely describe commands or "
        "leave safe in-scope work for the user. Before finishing, verify the result in proportion "
        "to risk and clearly report completed work, evidence, and any remaining limitation."
    ),
}


def apply_mode_defaults(snapshot: dict, mode: str) -> dict:
    """Fill missing budgets without overriding an Agent's explicit configuration."""
    defaults = MODE_DEFAULTS.get(mode, MODE_DEFAULTS["chat"])
    return {**defaults, **snapshot}


def mode_prompt(mode: str) -> str:
    return MODE_PROMPTS.get(mode, MODE_PROMPTS["chat"])
