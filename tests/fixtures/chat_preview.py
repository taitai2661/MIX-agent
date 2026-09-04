"""Disposable browser QA app. No real provider, runner, or existing user data.

Run with Docker Compose:
  docker compose --profile preview up --build preview
Sign in with preview / preview-only-password.
"""

import asyncio
import json
import os
from pathlib import Path

root = Path("/tmp/mix-chat-preview")
os.environ["MIX_DATA"] = str(root)
os.environ["MIX_KEYS"] = str(root / "keys")
os.environ["DATABASE_URL"] = "postgresql+psycopg://mix:test-only-password@preview-db/mix"
os.environ["PUBLIC_ORIGIN"] = "http://127.0.0.1:18080"

from fastapi import FastAPI
from fastapi.responses import StreamingResponse
from mix_agent.main import app
from mix_agent.db.models import Base, User, Provider, Model, Settings
from mix_agent.db.session import engine, SessionLocal
from mix_agent.auth.security import passwords, store_secret
from mix_agent.runs import engine as runs

# This profile's PostgreSQL database is disposable; reset it before loading
# deterministic preview data so restarting the service cannot reuse a user.
Base.metadata.drop_all(engine)
Base.metadata.create_all(engine)

with SessionLocal() as db:
    user = User(
        username="preview", password_hash=passwords.hash("preview-only-password")
    )
    db.add(user)
    db.flush()
    provider = Provider(
        owner_id=user.id,
        data={
            "kind": "openai",
            "base_url": "http://127.0.0.1:18080/fixture",
            "name": "Local test fixture",
        },
    )
    db.add(provider)
    db.flush()
    provider.data = {
        **provider.data,
        "secret_id": store_secret(db, user.id, "preview-only-key", "provider"),
    }
    model = Model(
        owner_id=user.id,
        data={
            "provider_id": provider.id,
            "model_id": "preview",
            "name": "検証専用モデル（実AIではありません）",
            "capabilities": {"reasoning": True, "tools": True},
        },
    )
    db.add(model)
    db.flush()
    db.add(
        Model(
            owner_id=user.id,
            data={
                "provider_id": provider.id,
                "model_id": "plain",
                "name": "未対応モデル（検証用）",
                "capabilities": {},
            },
        )
    )
    db.add(
        Settings(
            id="settings",
            owner_id=user.id,
            data={"setup_complete": True, "default_model_id": model.id},
        )
    )
    db.commit()


async def fake_execute(db, run, tool, args):
    return {
        "preview_only": True,
        "result": "検証用の結果です。実際の検索・ファイル変更・コマンド実行は行っていません。",
    }


async def fake_runner(*args, **kwargs):
    return {"preview_only": True}


runs.execute = fake_execute
runs.runner_request = fake_runner

fixture = FastAPI()


@fixture.post("/responses")
async def responses(body: dict):
    last = body["input"][-1]
    followup = last.get("type") == "function_call_output"
    content = str(last.get("content", ""))
    call = None
    if not followup and body.get("tools"):
        if "検索" in content:
            call = ("web_search", {"query": "test"})
        elif "作成" in content:
            call = ("write_file", {"path": "preview.txt", "content": "preview"})

    async def events():
        if body.get("reasoning"):
            yield (
                "data: "
                + json.dumps(
                    {
                        "type": "response.reasoning_summary_text.delta",
                        "delta": "依頼を確認し、必要な手順を検討しました。（検証用要約）",
                    },
                    ensure_ascii=False,
                )
                + "\n\n"
            )
            await asyncio.sleep(0.2)
        if call:
            output = [
                {
                    "type": "function_call",
                    "id": "fc",
                    "call_id": "call",
                    "name": call[0],
                    "arguments": json.dumps(call[1]),
                    "status": "completed",
                }
            ]
        else:
            text = "検証用の回答です。" + (
                "ツールの結果を確認しました。" if followup else ""
            )
            yield (
                "data: "
                + json.dumps(
                    {"type": "response.output_text.delta", "delta": text},
                    ensure_ascii=False,
                )
                + "\n\n"
            )
            output = [
                {
                    "type": "message",
                    "id": "msg",
                    "role": "assistant",
                    "status": "completed",
                    "content": [
                        {"type": "output_text", "text": text, "annotations": []}
                    ],
                }
            ]
        yield (
            "data: "
            + json.dumps(
                {
                    "type": "response.completed",
                    "response": {
                        "id": "r",
                        "object": "response",
                        "created_at": 0,
                        "model": "preview",
                        "status": "completed",
                        "output": output,
                        "usage": None,
                    },
                },
                ensure_ascii=False,
            )
            + "\n\n"
        )

    return StreamingResponse(events(), media_type="text/event-stream")


# Insert before the production app's SPA catch-all, only in this fixture process.
from starlette.routing import Mount

app.router.routes.insert(0, Mount("/fixture", app=fixture))
