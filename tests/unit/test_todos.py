from mix_agent.todos import derive, render_for_prompt, sanitize


def test_derive_marks_first_pending_in_progress_and_rest_pending():
    items = derive({"steps": ["a", "b", "c"], "pending": ["b", "c"]})
    assert items == [
        {"content": "a", "status": "completed"},
        {"content": "b", "status": "in_progress"},
        {"content": "c", "status": "pending"},
    ]


def test_derive_all_completed_when_pending_is_empty():
    items = derive({"steps": ["a", "b"], "pending": []})
    assert [item["status"] for item in items] == ["completed", "completed"]


def test_derive_keeps_unmatched_pending_entries_as_pending():
    items = derive({"steps": ["a"], "pending": ["paraphrased"]})
    assert items == [
        {"content": "a", "status": "completed"},
        {"content": "paraphrased", "status": "pending"},
    ]


def test_derive_handles_empty_and_malformed_plan():
    assert derive(None) == []
    assert derive({}) == []
    assert derive({"steps": ["a"], "pending": None}) == [
        {"content": "a", "status": "completed"},
    ]
    assert derive({"steps": [None, 3, " a ", "a"], "pending": ["a"]}) == [
        {"content": "a", "status": "in_progress"},
    ]


def test_derive_deduplicates_and_caps_items():
    plan = {"steps": [f"step {index}" for index in range(40)], "pending": []}
    items = derive(plan)
    assert len(items) == 30
    assert len({item["content"] for item in items}) == 30
    long = derive({"steps": ["x" * 500], "pending": ["x" * 500]})
    assert len(long[0]["content"]) == 300


def test_sanitize_drops_malformed_entries():
    raw = [
        {"content": "ok", "status": "pending"},
        {"content": "  ", "status": "pending"},
        {"content": "bad", "status": "done"},
        {"content": 42, "status": "pending"},
        "not-a-dict",
        {"content": "y", "status": "completed"},
    ]
    assert sanitize(raw) == [
        {"content": "ok", "status": "pending"},
        {"content": "y", "status": "completed"},
    ]
    assert sanitize(None) == []
    assert sanitize("junk") == []


def test_render_for_prompt_lists_marks_and_returns_empty_without_items():
    assert render_for_prompt([]) == ""
    assert render_for_prompt(None) == ""
    text = render_for_prompt([
        {"content": "phase:setup", "status": "completed"},
        {"content": "edit", "status": "in_progress"},
        {"content": "verify", "status": "pending"},
    ])
    assert "update_plan.steps" in text
    assert "- [x] phase:setup" in text
    assert "- [~] edit" in text
    assert "- [ ] verify" in text
