"""ChatGPT-style context rendering: flat bullet list."""

from __future__ import annotations

from mix_agent.context import builder as context_builder


def _window(window, reserved=4096):
    return {"context_window": window, "reserved_output_tokens": reserved, "safety_margin": 1000}


def _memory(content: str, mid: str) -> dict:
    return {"id": mid, "content": content, "gist": content[:200]}


def test_context_renders_flat_memory_list():
    memories = [
        _memory("回答は日本語にする", "m1"),
        _memory("MIX-agent は FastAPI を使う", "m2"),
    ]
    built = context_builder.build_initial(
        system_text="sys",
        prior_messages=[],
        current_message={"role": "user", "content": "release を整えたい"},
        memories=memories,
        window_info=_window(128_000),
        model_id="test",
        trigger="interactive",
    )
    head = built["messages"][0]["content"]
    assert "回答は日本語にする" in head
    assert "MIX-agent は FastAPI を使う" in head


def test_context_memory_block_counts_all():
    memories = [_memory("doc1", "a"), _memory("doc2", "b")]
    built = context_builder.build_initial(
        system_text="sys",
        prior_messages=[],
        current_message={"role": "user", "content": "hello"},
        memories=memories,
        window_info=_window(32_000),
        model_id="test",
    )
    groups = built["trace"]["included"]["memory_groups"]
    assert groups["all"] == 2
