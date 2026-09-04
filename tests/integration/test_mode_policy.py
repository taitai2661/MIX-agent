from mix_agent.runs.mode_policy import apply_mode_defaults, mode_prompt, mode_policy, tool_allowed


def test_each_mode_has_a_distinct_execution_policy():
    prompts = {mode: mode_prompt(mode) for mode in ("chat", "thinking", "agent")}
    assert len(set(prompts.values())) == 3
    assert "immediate request" in prompts["chat"]
    assert "cross-check" in prompts["thinking"]
    assert "update_plan" in prompts["agent"]
    assert "observable success" in prompts["agent"]


def test_mode_defaults_expand_autonomous_work_budgets():
    chat = apply_mode_defaults({}, "chat")
    thinking = apply_mode_defaults({}, "thinking")
    agent = apply_mode_defaults({}, "agent")
    assert chat["max_seconds"] < thinking["max_seconds"] < agent["max_seconds"]
    assert chat["max_tool_calls"] < thinking["max_tool_calls"] < agent["max_tool_calls"]
    assert chat["max_steps"] < thinking["max_steps"] < agent["max_steps"]
    assert not chat["policy"]["planning"]
    assert not thinking["policy"]["checkpointing"]
    assert agent["policy"]["planning"]
    assert agent["policy"]["background_processes"]


def test_explicit_agent_limits_are_preserved():
    configured = {"max_steps": 7, "max_seconds": 45, "max_tool_calls": 9}
    applied = apply_mode_defaults(configured, "agent")
    assert {key: applied[key] for key in configured} == configured
    assert applied["policy"]["max_steps"] == 7


def test_short_modes_cannot_inherit_long_work_limits():
    configured = {"max_steps": 200, "max_seconds": 3600, "max_tool_calls": 500}
    assert apply_mode_defaults(configured, "chat")["max_steps"] == 12
    assert apply_mode_defaults(configured, "thinking")["max_tool_calls"] == 24


def test_mode_restrictions_cannot_be_bypassed_with_arguments():
    plan = {"id": "update_plan", "allowed_modes": ["agent"]}
    terminal = {"id": "run_terminal", "allowed_modes": ["chat", "thinking", "agent"]}
    assert not tool_allowed("chat", plan)
    assert tool_allowed("agent", plan)
    assert not tool_allowed("thinking", terminal, {"background": True})
    assert tool_allowed("agent", terminal, {"background": True})
    assert mode_policy("chat")["label"] == "通常"
