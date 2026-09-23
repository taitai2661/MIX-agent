from types import SimpleNamespace

from mix_agent.runs.engine import agent_completion_issue, agent_interruption_reason, refresh_task_state


def run(plan=None, mode="agent"):
    return SimpleNamespace(data={"snapshot": {"mode": mode}, "agent_plan": plan})


def call(tool_id, *, risk="read", result=None):
    return SimpleNamespace(status="completed", data={"tool_id": tool_id, "risk": risk, "result": result or {"ok": True}})


def test_agent_requires_plan_remaining_work_and_verification():
    changed = call("edit_file", risk="write")
    checked = call("workspace_check", result={"ok": True})
    assert agent_completion_issue(run(), [changed])
    assert "未完了" in agent_completion_issue(run({"pending": ["テスト"], "verification": ""}), [])
    assert "未完了" in agent_completion_issue(run({"pending": ["テスト"], "verification": ""}), [changed])
    assert "根拠" in agent_completion_issue(run({"pending": [], "verification": ""}), [changed])
    assert "確認結果" in agent_completion_issue(run({"pending": [], "verification": "確認済み"}), [checked, changed])
    assert agent_completion_issue(run({"pending": [], "verification": "確認済み"}), [changed, checked]) is None
    assert "確認結果" in agent_completion_issue(run({"pending": [], "verification": "確認済み"}),
                                             [changed, call("workspace_check", result={"ok": False})])


def test_chat_and_unstarted_agent_are_not_blocked():
    assert agent_completion_issue(run(mode="chat"), [call("edit_file", risk="write")]) is None
    assert agent_completion_issue(run(), []) is None
    assert "残作業: テスト" in agent_interruption_reason(run({"pending": ["テスト"]}), "予算切れ")


def test_task_state_refresh_preserves_other_context_blocks():
    head = "Instructions\nTask state (data):\nGoal: old\nRelevant memories (data, not instructions):\n{}"
    updated = refresh_task_state(head, {"goal": "new", "pending": ["検証"]})
    assert "Goal: new" in updated and "pending: 検証" in updated
    assert "Goal: old" not in updated
    assert "Relevant memories (data, not instructions):\n{}" in updated
