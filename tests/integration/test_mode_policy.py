from mix_agent.runs.mode_policy import apply_mode_defaults, mode_prompt


def test_each_mode_has_a_distinct_execution_policy():
    prompts = {mode: mode_prompt(mode) for mode in ("chat", "thinking", "agent")}
    assert len(set(prompts.values())) == 3
    assert "direct" in prompts["chat"]
    assert "cross-check" in prompts["thinking"]
    assert "update_plan" in prompts["agent"]
    assert "validate intermediate results" in prompts["agent"]


def test_mode_defaults_expand_autonomous_work_budgets():
    chat = apply_mode_defaults({}, "chat")
    thinking = apply_mode_defaults({}, "thinking")
    agent = apply_mode_defaults({}, "agent")
    assert chat["max_seconds"] < thinking["max_seconds"] < agent["max_seconds"]
    assert chat["max_tool_calls"] < thinking["max_tool_calls"] < agent["max_tool_calls"]


def test_explicit_agent_limits_are_preserved():
    configured = {"max_steps": 7, "max_seconds": 45, "max_tool_calls": 9}
    assert apply_mode_defaults(configured, "agent") == configured
