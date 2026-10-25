"""Phase 2 extraction quality: spec §6 prompt, input/output contract, commands.

The contract is ``docs/memory-spec.md`` §6 (and §8 for secrets). Nothing in
this file touches the network: the provider is a monkeypatched
``providers.adapters.Adapter`` that records the request and answers with a
canned JSON body, so the extractor payload can be asserted directly.
"""

from __future__ import annotations

import ast
import json
import re
from datetime import timedelta
from pathlib import Path
from types import SimpleNamespace

from sqlalchemy import func, select

from mix_agent.db.models import (
    Conversation,
    Memory,
    MemoryProcessingJob,
    Provider,
    Run,
    User,
    now,
)
from mix_agent.db.session import SessionLocal
from mix_agent.memory import jobs, service
from mix_agent.runs import engine as runs_engine

# Same shape as scripts/check-memory-parity.py's §6 reader: parent of `tests/`
# is the edition root both on the host (docker/MIX-agent) and in the test
# image (/workspace), where the Dockerfile copies `docs/` for exactly this.
SPEC_PATH = Path(__file__).resolve().parents[2] / "docs" / "memory-spec.md"
EXTRACTION_PROMPT_BLOCK = re.compile(r"\*\*共通抽出プロンプト\*\*:\s*\n\n```\n(.*?)\n```", re.DOTALL)


def owner_id(db):
    return db.scalar(select(User.id))


def memory_count(db, owner):
    return db.scalar(select(func.count()).select_from(Memory).where(Memory.owner_id == owner))


def test_extraction_prompt_is_verbatim_spec_section_6():
    """The prompt is a published contract, not an implementation detail.

    scripts/check-memory-parity.py compares the same pair on the host and in
    CI; reading the spec here means a spec edit that forgets the Docker
    implementation (or the other way round) fails the test suite too.
    """
    assert SPEC_PATH.is_file(), f"memory spec missing at {SPEC_PATH}"
    match = EXTRACTION_PROMPT_BLOCK.search(SPEC_PATH.read_text(encoding="utf-8"))
    assert match, "spec §6 has no 共通抽出プロンプト block"
    assert jobs.EXTRACTION_PROMPT.strip() == match.group(1).strip()


def test_extraction_prompt_is_one_ast_string_literal():
    """Parity reads the constant with `ast`; an f-string, a concatenation or a
    `.format()` call would make the checker report the prompt as missing."""
    source = Path(jobs.__file__).read_text(encoding="utf-8")
    value = None
    for node in ast.parse(source).body:
        if isinstance(node, ast.Assign) and any(
            isinstance(target, ast.Name) and target.id == "EXTRACTION_PROMPT" for target in node.targets
        ):
            value = node.value
            break
    assert isinstance(value, ast.Constant) and isinstance(value.value, str)


def test_payload_defaults_missing_role_and_action_to_fact_create():
    """An item without `role`/`action` must not be dropped (§6 output contract)."""
    items = jobs._parse_payload('{"memories": [{"content": "ユーザーは日本語を好む"}]}')
    assert len(items) == 1
    assert items[0]["action"] == "CREATE"
    assert items[0]["role"] == "fact"
    assert items[0]["content"] == "ユーザーは日本語を好む"

    # The legacy bare-string shape is defaulted too.
    items = jobs._parse_payload('{"memories": ["回答は日本語にする"]}')
    assert items[0]["action"] == "CREATE"
    assert items[0]["role"] == "fact"

    # Values that are present are kept verbatim (lower-cased role).
    items = jobs._parse_payload(
        '{"memories": [{"content": "OSSを優先する", "role": "Preference", "action": "create", "reason": "stated"}]}'
    )
    assert items[0] == {"action": "CREATE", "role": "preference", "content": "OSSを優先する", "reason": "stated"}


def test_unsafe_extraction_items_are_no_op_not_errors():
    """§6/§8: secret, >200 chars, out-of-vocabulary role/action -> NO_OP."""
    items = jobs._parse_payload(json.dumps({"memories": [
        {"content": "ユーザーは Vegan", "role": "preference", "action": "CREATE"},
        {"content": "api_key=sk-not-a-memory", "role": "fact", "action": "CREATE"},
        {"content": "あ" * 201, "role": "fact", "action": "CREATE"},
    ]}, ensure_ascii=False))
    assert [item["action"] for item in items] == ["CREATE", "NO_OP", "NO_OP"]

    items = jobs._parse_payload(json.dumps({"memories": [
        {"content": "語彙外のロール", "role": "opinion", "action": "CREATE"},
        {"content": "未知のアクション", "role": "fact", "action": "UPDATE"},
        {"content": "明示的に捨てる", "role": "fact", "action": "NO_OP"},
    ]}, ensure_ascii=False))
    assert [item["action"] for item in items] == ["NO_OP", "NO_OP", "NO_OP"]


def test_no_op_items_are_never_persisted(signed):
    """A NO_OP candidate costs nothing: no row, and the good item still lands."""
    items = jobs._parse_payload(json.dumps({"memories": [
        {"content": "ユーザーは Vegan", "role": "preference", "action": "CREATE"},
        {"content": "api_key=sk-not-a-memory", "role": "fact", "action": "CREATE"},
    ]}, ensure_ascii=False))
    with SessionLocal() as db:
        owner = owner_id(db)
        job = SimpleNamespace(owner_id=owner, run_id=None)
        touched = jobs._apply(db, job, items)
        db.commit()
        rows = list(db.scalars(select(Memory).where(Memory.owner_id == owner)))
        assert memory_count(db, owner) == 1
        stored = rows[0]
        assert "api_key" not in stored.content
        # §1's `role` lives in the row's data (Docker has no taxonomy column).
        assert stored.data.get("role") == "preference"
        assert service.role_value(stored.data.get("role")) == "preference"
        assert [row.id for row in rows] == touched


def test_reinforce_refreshes_one_row_and_creates_nothing(signed):
    with SessionLocal() as db:
        owner = owner_id(db)
        created = service.change(
            db, owner, "Postgres のマイグレーションはDocker経由で実施する", source_run="explicit-user-request"
        )
        row = db.get(Memory, created["id"])
        stale = now() - timedelta(days=180)
        row.updated_at = stale
        db.commit()
        before = memory_count(db, owner)

        job = SimpleNamespace(owner_id=owner, run_id=None)
        touched = jobs._apply(db, job, [{
            "action": "REINFORCE",
            "role": "fact",
            "content": "Postgres のマイグレーションはDocker経由で実施する",
            "reason": "said again",
        }])
        db.commit()
        assert touched == [created["id"]]
        assert memory_count(db, owner) == before
        refreshed = db.get(Memory, created["id"])
        assert refreshed.updated_at > stale

        # A REINFORCE that matches nothing must not fall back to CREATE.
        touched = jobs._apply(db, job, [{
            "action": "REINFORCE",
            "role": "fact",
            "content": "zorkmids are fictional",
            "reason": "hallucinated repeat",
        }])
        db.commit()
        assert touched == []
        assert memory_count(db, owner) == before


def test_reinforce_picks_the_lexically_matching_row_not_the_newest(signed):
    """§5's recency term alone must not choose the target (see jobs._reinforce_target)."""
    with SessionLocal() as db:
        owner = owner_id(db)
        target = service.change(db, owner, "回答は日本語にする", source_run="explicit-user-request")
        # A newer, unrelated row: recency alone would rank it first.
        service.change(db, owner, "zorkmids are fictional", source_run="explicit-user-request")
        db.commit()

        job = SimpleNamespace(owner_id=owner, run_id=None)
        touched = jobs._apply(db, job, [{
            "action": "REINFORCE", "role": "fact",
            "content": "日本語で回答する", "reason": "repeat",
        }])
        db.commit()
        assert touched == [target["id"]]


async def test_extraction_payload_carries_existing_top10(signed, monkeypatch):
    """§6 input contract: {"user", "assistant", "existing"} + §6 system prompt."""
    captured = {}

    class RecordingAdapter:
        def __init__(self, provider, key):
            self.provider = provider

        async def stream(self, model, messages, tools, mode, settings):
            captured["model"] = model
            captured["messages"] = messages
            body = json.dumps({"memories": [{
                "content": "ユーザーは Vegan",
                "role": "preference",
                "action": "CREATE",
                "reason": "stated preference",
            }]}, ensure_ascii=False)
            yield {"kind": "response", "message": {"role": "assistant", "content": body}}

    monkeypatch.setattr("mix_agent.providers.adapters.Adapter", RecordingAdapter)

    with SessionLocal() as db:
        owner = owner_id(db)
        service.change(db, owner, "回答は日本語にする", source_run="explicit-user-request")
        service.change(db, owner, "MIX Providerの設計", source_run="explicit-user-request")
        provider = Provider(owner_id=owner, data={"kind": "compatible", "base_url": "https://example.invalid/v1"})
        conversation = Conversation(owner_id=owner, data={"title": "extraction"})
        db.add_all([provider, conversation])
        db.flush()
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key="extraction-quality-payload",
            data={
                "snapshot": {"provider_record_id": provider.id, "model_id": "mix-test-model", "mode": "chat"},
                "history": [{"role": "user", "content": "回答は日本語にする"}],
                "steps": 0,
                "tool_count": 0,
            },
        )
        db.add(run)
        db.flush()
        job = MemoryProcessingJob(
            owner_id=owner,
            run_id=run.id,
            status="pending",
            data={
                "user_content": "回答は日本語にする",
                "assistant_content": "はい、日本語で回答します",
                "next_attempt_at": now().isoformat(),
            },
        )
        db.add(job)
        db.commit()
        job_id = job.id

    await jobs.process_one(job_id)

    assert captured, "the extractor provider was never called"
    messages = captured["messages"]
    assert messages[0] == {"role": "system", "content": jobs.EXTRACTION_PROMPT}
    payload = json.loads(messages[1]["content"])
    assert set(payload) == {"user", "assistant", "existing"}
    assert payload["user"] == "回答は日本語にする"
    assert payload["assistant"] == "はい、日本語で回答します"
    assert sorted(payload["existing"]) == sorted(["回答は日本語にする", "MIX Providerの設計"])
    assert len(payload["existing"]) <= 10

    with SessionLocal() as db:
        job = db.get(MemoryProcessingJob, job_id)
        assert job.status == "completed", job.data.get("error")
        owner = owner_id(db)
        rows = [row for row in db.scalars(select(Memory).where(Memory.owner_id == owner)) if "Vegan" in row.content]
        assert len(rows) == 1
        assert rows[0].data.get("role") == "preference"


def test_explicit_candidates_cover_politeness_language_and_punctuation():
    # Regression: the two original patterns keep their exact matches.
    assert service.explicit_candidate("覚えて: 回答は日本語で簡潔に") == "回答は日本語で簡潔に"
    assert service.explicit_candidate("回答は日本語で簡潔にするを覚えておいて") == "回答は日本語で簡潔にする"
    assert service.explicit_candidate("これを覚えて") is None
    assert service.explicit_candidate("覚えて: api_key=private") is None
    # 敬語・命令形の揺れ (prefix needs a separator: 「覚えてる？」 is not a command).
    assert service.explicit_candidate("覚えておいてください：明日の天気は東京で聞く") == "明日の天気は東京で聞く"
    assert service.explicit_candidate("覚えといて：プロジェクト名はMIX-agent") == "プロジェクト名はMIX-agent"
    assert service.explicit_candidate("メモして：接続先はVPN") == "接続先はVPN"
    assert service.explicit_candidate("回答は日本語で覚えてください。") == "回答は日本語で"
    assert service.explicit_candidate("明日の会議は10時だとメモして") == "明日の会議は10時だと"
    assert service.explicit_candidate("记住：VPN経由で接続") == "VPN経由で接続"
    # 英語
    assert service.explicit_candidate("remember that I prefer dark mode") == "I prefer dark mode"
    assert service.explicit_candidate("note that the office is in Osaka") == "the office is in Osaka"
    assert service.explicit_candidate("don't forget the standup at 9") == "the standup at 9"
    assert service.explicit_candidate("please remember to call mom") == "call mom"
    assert service.explicit_candidate("buy milk, don't forget") == "buy milk"
    # 句読点・空白の揺れ
    assert service.explicit_candidate("  覚えて：  回答は日本語で  ") == "回答は日本語で"
    assert service.explicit_candidate("　覚えておいて　明日の会議は10時　") == "明日の会議は10時"
    assert service.explicit_candidate("スペイン語を覚えておいて？") == "スペイン語"
    assert service.explicit_candidate("明日は雨だと覚えておいて！") == "明日は雨だと"


def test_explicit_forget_candidates_cover_politeness_language_and_punctuation():
    # Regression: the original pattern keeps its exact match.
    assert service.explicit_forget_candidate("Chromeをメインに使うを忘れて") == "Chromeをメインに使う"
    assert service.explicit_forget_candidate("忘れてください：Chrome") == "Chrome"
    assert service.explicit_forget_candidate("Chromeを忘れてください。") == "Chrome"
    assert service.explicit_forget_candidate("記憶から消してください、旧パス") == "旧パス"
    assert service.explicit_forget_candidate("forget the wifi password") == "the wifi password"
    assert service.explicit_forget_candidate("forget about the legacy key") == "the legacy key"
    assert service.explicit_forget_candidate("forget it") is None
    assert service.explicit_forget_candidate("remember this") is None
    # A file operation is not a memory command: only memory-specific verbs match.
    assert service.explicit_forget_candidate("このファイルを消して") is None


def test_explicit_commands_are_applied_by_the_message_path(signed):
    """End-to-end: an explicit save goes through routes without a model call."""
    from mix_agent.api import routes

    with SessionLocal() as db:
        owner = owner_id(db)
        routes._handle_explicit_memory_requests(db, owner, "覚えておいてください：回答は日本語で", False)
        routes._handle_explicit_memory_requests(db, owner, "回答は日本語でを忘れてください。", False)
        db.commit()
        assert memory_count(db, owner) == 0

        routes._handle_explicit_memory_requests(db, owner, "覚えておいてください：回答は日本語で", False)
        db.commit()
        assert memory_count(db, owner) == 1


def test_synchronous_run_observation_harvest_stays_disabled():
    """Recorded decision: no sync harvesting (see the function's docstring for
    the reasoning: spec §0/§1/§3 put task-scoped memory behind Phase 3)."""
    assert runs_engine._collect_memory_observations(None, None, "final answer") == []
