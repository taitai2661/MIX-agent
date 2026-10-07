"""Tests for the role-organized Memory rendering in ContextBuilder.

These tests verify that the Context Engine surfaces role-bucketed memory
sections (Decisions, Failures, Facts, etc.) instead of the legacy flat
"Relevant memories" list.
"""

from __future__ import annotations

from mix_agent.context import builder as context_builder
from mix_agent.memory import types as mem_types


def _window(window, reserved=4096):
    return {"context_window": window, "reserved_output_tokens": reserved, "safety_margin": 1000}


def _memory(role: str, scope: str, content: str, **extra) -> dict:
    base = {
        "id": f"m-{role}-{content[:4]}",
        "role": role,
        "scope": scope,
        "content": content,
        "gist": content[:200],
        "lifecycle": mem_types.LIFECYCLE_ACTIVE,
        "confidence": 0.9,
        "salience": 0.6,
        "entities": [],
        "concepts": [],
    }
    base.update(extra)
    return base


def test_context_renders_memory_in_role_buckets():
    memories = [
        _memory(mem_types.ROLE_DECISION, mem_types.SCOPE_PROJECT, "Docker で release を分離する",
                role_metadata={"decision": "Docker", "reason": "Desktop 依存を切るため", "status": "active"}),
        _memory(mem_types.ROLE_FAILURE, mem_types.SCOPE_PROJECT, "alembic upgrade head を直接実行すると失敗",
                role_metadata={"attempt": "alembic upgrade head", "outcome": "failed", "reason": "env var missing"}),
        _memory(mem_types.ROLE_PREFERENCE, mem_types.SCOPE_USER, "回答は日本語にする"),
        _memory(mem_types.ROLE_FACT, mem_types.SCOPE_WORLD, "MIX-agent は FastAPI を使う"),
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
    assert "Relevant active Decisions" in head
    assert "Past failures to avoid" in head
    assert "User preferences" in head
    assert "Relevant facts" in head
    # The decision body should appear in its own bucket.
    assert "Docker で release を分離する" in head


def test_context_memory_block_tracks_bucket_counts():
    memories = [
        _memory(mem_types.ROLE_DECISION, mem_types.SCOPE_USER, "doc1"),
        _memory(mem_types.ROLE_DECISION, mem_types.SCOPE_USER, "doc2"),
        _memory(mem_types.ROLE_FAILURE, mem_types.SCOPE_USER, "fail"),
    ]
    built = context_builder.build_initial(
        system_text="sys",
        prior_messages=[],
        current_message={"role": "user", "content": "hello"},
        memories=memories,
        window_info=_window(32_000),
        model_id="test",
    )
    groups = built["trace"]["included"]["memory_groups"]
    assert groups["decisions"] == 2
    assert groups["failures"] == 1
    assert groups["facts"] == 0
