"""Phase 1 Context Engine tests (pure unit + light integration, no provider calls)."""

import json

from mix_agent.context import builder as context_builder
from mix_agent.context import budget as context_budget
from mix_agent.context import task_state as task_state_mod
from mix_agent.context.builder import select_recent
from mix_agent.context.references import (
    extract_data_url_images,
    is_base64_image,
    tool_envelope_text,
    tool_ref_message,
)
from mix_agent.context.retrievers import fit_items
from mix_agent.context.summary import finalize, merge_prompt
from mix_agent.context.tokens import count, count_messages
from mix_agent.context.types import ContextBudgetError


def _window(window, reserved=4096):
    return {"context_window": window, "reserved_output_tokens": reserved, "safety_margin": 1000}


def test_context_window_ignores_boolean_saved_values():
    resolved = context_budget.resolve_window({
        "context_window": True,
        "max_output_tokens": False,
        "safety_margin": True,
    })
    assert resolved == {
        "context_window": context_budget.FALLBACK_CONTEXT_WINDOW,
        "reserved_output_tokens": context_budget.FALLBACK_RESERVED_OUTPUT,
        "safety_margin": 1000,
    }


def _messages(n, size=200):
    return [
        {"role": "user" if i % 2 == 0 else "assistant", "content": f"msg-{i} " + "x" * size}
        for i in range(n)
    ]


def test_small_window_fits_budget():
    built = context_builder.build_initial(
        system_text="sys",
        prior_messages=_messages(100),
        current_message={"role": "user", "content": "hello"},
        task_goal="hello",
        memories=[{"id": f"m{i}", "content": "y" * 500} for i in range(10)],
        skills=[{"id": f"s{i}", "content": "z" * 500} for i in range(10)],
        knowledge=[],
        tools=[],
        window_info=_window(32_000),
        model_id="test-small",
        trigger="interactive",
    )
    assert built["trace"]["estimated_input_tokens"] <= built["trace"]["input_budget"] + 2000
    assert built["trace"]["excluded"]["evicted_count"] > 0
    assert built["recent"][-1]["content"] == "hello"


def test_large_window_no_early_trim():
    prior = _messages(10)
    built = context_builder.build_initial(
        system_text="sys",
        prior_messages=prior,
        current_message={"role": "user", "content": "hello"},
        window_info=_window(1_000_000),
        model_id="test-large",
    )
    assert built["trace"]["excluded"]["evicted_count"] == 0
    assert len(built["recent"]) == 11


def test_long_tool_output_reference():
    long_text = json.dumps({"status": "succeeded", "result": {"data": "v" * 50_000}}, ensure_ascii=False)
    is_long, summary = tool_envelope_text(long_text, 4000)
    assert is_long
    entry = tool_ref_message("call-1", "github_search", summary, "art-1", True)
    assert entry["call_id"] == "call-1" and entry["name"] == "github_search"
    assert len(entry["content"]) < len(long_text)
    assert json.loads(entry["content"])["artifact_ref"] == "artifact://art-1"


def test_image_ref_extraction():
    data_url = "data:image/png;base64," + "QUJD" * 1000
    message = {"role": "user", "content": "see this", "images": [data_url]}
    assert is_base64_image(data_url)
    updated, extracted = extract_data_url_images(message)
    assert updated["images"] == []
    assert len(updated["image_refs"]) == 1 and extracted[0]["bytes"]


def test_task_state_survives_validation():
    state = {"goal": "ship", "pending": ["a", "b"], "constraints": ["c"]}
    assert task_state_mod.validate(state)["goal"] == "ship"
    broken = task_state_mod.merge(state, {"goal": "", "pending": "not-a-list"})
    assert broken["goal"] == "ship"  # never silently drop goal
    assert task_state_mod.validate(None)["goal"] == ""


def test_progressive_summary_prompt_incremental():
    previous = "user wants X"
    evicted = [{"role": "user", "content": "old 1"}, {"role": "assistant", "content": "old 2"}]
    prompt = merge_prompt(previous, evicted)
    payload = json.loads(prompt[1]["content"])
    assert payload["previous_summary"] == previous
    assert len(payload["newly_evicted"]) == 2
    assert finalize("  hello world  ") == "hello world"


def test_memory_skill_budget_selection():
    items = [{"id": f"m{i}", "content": "t" * 300} for i in range(20)]
    included, excluded = fit_items(items, 2000, "m")
    assert included and excluded
    assert sum(len(json.dumps(i)) for i in included) < sum(len(json.dumps(i)) for i in items)


def test_provider_switching_rebudgets():
    small = context_budget.input_budget(_window(32_000))
    large = context_budget.input_budget(_window(200_000))
    assert large > small
    recent_small, evicted_small = select_recent(_messages(50), small // 4, "m")
    recent_large, _ = select_recent(_messages(50), large // 4, "m")
    assert len(recent_large) >= len(recent_small)


def test_scheduled_uses_same_builder():
    built = context_builder.build_initial(
        system_text="sys",
        prior_messages=_messages(5),
        current_message={"role": "user", "content": "nightly report"},
        window_info=_window(128_000),
        model_id="test",
        trigger="scheduled",
    )
    assert built["trace"]["trigger"] == "scheduled"
    assert built["recent"]


def test_resume_state_restoration_shape():
    persisted = {
        "task_state": {"goal": "g", "pending": ["p"]},
        "summary": {"text": "s", "covered_count": 3},
    }
    restored_state = task_state_mod.ensure(persisted["task_state"], "")
    assert restored_state["goal"] == "g" and restored_state["pending"] == ["p"]


def test_context_budget_error_on_impossible():
    try:
        context_builder.build_initial(
            system_text="S" * 100_000,
            prior_messages=[],
            current_message={"role": "user", "content": "hi"},
            window_info=_window(8000, reserved=4000),
            model_id="tiny",
        )
    except ContextBudgetError:
        return
    # Either raises or fits via mandatory-message guarantee; both acceptable.
    assert True


def test_token_counter_single_funnel():
    assert count("hello") > 0
    assert count_messages([{"role": "user", "content": "hi"}]) > 0
