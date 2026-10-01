from mix_agent.db.models import Conversation, Run, ToolCall, User, uid
from mix_agent.db.session import SessionLocal
from mix_agent.runs import engine
from sqlalchemy import select


def _tool(name="write_file"):
    return {
        "id": name, "model_name": name, "description": "t",
        "source": "builtin", "executor_ref": "builtin",
        "default_permission": "ask", "risk": "approval",
        "input_schema": {"type": "object", "properties": {}, "additionalProperties": False},
        "allowed_modes": ["chat", "thinking", "agent"],
    }


def _make_run():
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "Artifacts"})
        db.add(conversation)
        db.flush()
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key=str(uid()),
            data={
                "snapshot": {
                    "mode": "agent", "model_id": "fake",
                    "provider": {"kind": "compatible"},
                    "tool_ids": ["write_file"], "tools": [_tool()],
                },
                "history": [{"role": "user", "content": "Do the test"}],
            },
        )
        db.add(run)
        db.commit()
        return run.id


def _complete(db, run, result):
    call = ToolCall(
        owner_id=run.owner_id, run_id=run.id,
        data={"tool_id": "write_file", "name": "write_file",
              "risk": "approval", "provider_call_id": "c1", "arguments": {}},
    )
    db.add(call)
    db.flush()
    engine.complete_tool_call(db, run, call, _tool(), result)


def test_context_tool_output_artifact_stays_out_of_user_artifacts(signed):
    run_id = _make_run()
    artifact = {
        "artifact_id": "ctx-1", "name": "tool-result.json", "mime": "application/json",
        "size": 4, "kind": "context-tool-output",
    }
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        _complete(db, run, {
            "excerpt": "short excerpt", "truncated": True,
            "artifact": artifact, "artifact_ref": "artifact://ctx-1",
        })
        assert run.data.get("artifacts", []) == []
        assert "ctx-1" in run.data["tool_refs"]


def test_user_created_artifact_still_surfaces(signed):
    run_id = _make_run()
    artifact = {
        "artifact_id": "user-1", "name": "result.html", "mime": "text/html",
        "size": 5, "kind": "user-created",
    }
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        _complete(db, run, {"artifact": artifact})
        assert [item["artifact_id"] for item in run.data["artifacts"]] == ["user-1"]


def test_long_output_wrapper_artifact_is_internal_only(signed, tmp_path, monkeypatch):
    from mix_agent import config

    monkeypatch.setattr(config, "ARTIFACTS", tmp_path)
    run_id = _make_run()
    with SessionLocal() as db:
        run = db.get(Run, run_id)
        _complete(db, run, {"text": "x" * 10000})
        assert run.data.get("artifacts", []) == []
        ref = run.data["tool_refs"][-1]
        assert ref
        assert (tmp_path / ref).exists()
        history = run.data["history"][-1]
        assert history["role"] == "tool"
        assert history.get("tool_ref") == ref


def test_large_browser_read_artifact_not_exposed_in_tool_history(signed, tmp_path, monkeypatch):
    from mix_agent import config

    monkeypatch.setattr(config, "ARTIFACTS", tmp_path)
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "history"})
        db.add(conversation)
        db.flush()
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key=str(uid()),
            status="completed",
            data={"snapshot": {}, "history": [], "artifacts": []},
        )
        db.add(run)
        db.flush()
        internal = {
            "artifact_id": "ctx-2", "name": "tool-result.json", "mime": "application/json",
            "size": 4, "kind": "context-tool-output",
        }
        external = {
            "artifact_id": "user-2", "name": "page.html", "mime": "text/html",
            "size": 5, "kind": "user-created",
        }
        db.add_all([
            ToolCall(owner_id=owner, run_id=run.id, status="completed", data={
                "tool_id": "browser_read", "name": "browser_read",
                "result": {"excerpt": "x", "truncated": True, "artifact": internal,
                           "artifact_ref": "artifact://ctx-2"},
            }),
            ToolCall(owner_id=owner, run_id=run.id, status="completed", data={
                "tool_id": "create_artifact", "name": "create_artifact",
                "result": {"artifact": external},
            }),
        ])
        db.commit()
        conversation_id = conversation.id

    calls = signed.get(f"/api/v1/conversations/{conversation_id}/tool-calls").json()
    internal_call, external_call = calls
    assert internal_call["artifact"] is None
    assert external_call["artifact"]["artifact_id"] == "user-2"
    assert external_call["artifact"]["name"] == "page.html"