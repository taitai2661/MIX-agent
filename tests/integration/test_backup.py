import copy
from sqlalchemy import select
import pytest
from mix_agent.storage import backup
from mix_agent.db.session import SessionLocal
from mix_agent.db.models import User, Memory, Secret
from mix_agent.memory.service import change
from mix_agent.auth.security import store_secret, read_secret
from mix_agent.tools.execute import save_artifact
from mix_agent import config

@pytest.fixture
def fake_runners(monkeypatch):
    state = {"execution": {"files": {"hello.txt": "aGVsbG8="}}, "mcp": {"files": {}}, "manager": {"volumes": {}}}
    async def request(kind, path, payload, **kwargs):
        if path == "/snapshot":
            return copy.deepcopy(state[kind])
        if path == "/validate-snapshot":
            return {"ok": True}
        if path == "/restore-snapshot":
            state[kind] = copy.deepcopy(payload)
            return {"ok": True}
        if path == "/v1/validate-snapshot":
            return {"ok": True}
        if path == "/v1/restore-snapshot":
            state[kind] = copy.deepcopy(payload)
            return {"ok": True}
        raise AssertionError(path)
    monkeypatch.setattr(backup, "runner_request", request)
    return state

async def test_complete_backup_restores_db_key_files(signed, fake_runners):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        change(db, owner, "remember this")
        secret_id = store_secret(db, owner, "mock-provider-secret", "provider")
        artifact = save_artifact(db, owner, b"artifact", "example.txt")
        db.commit()
        raw = await backup.create(db, "test-backup-password")
        assert b"mock-provider-secret" not in raw
        for row in db.scalars(select(Memory)):
            row.data = {**row.data, "content": "changed"}
        db.commit()
        fake_runners["execution"] = {"files": {}}
        result = await backup.restore(db, raw, "test-backup-password")
        assert result["relogin_required"]
    with SessionLocal() as db:
        assert db.scalar(select(Memory)).data["content"] == "remember this"
        assert read_secret(db, secret_id) == ""
        assert (config.ARTIFACTS / artifact["artifact_id"]).read_bytes() == b"artifact"
    assert fake_runners["execution"]["files"]["hello.txt"] == "aGVsbG8="

async def test_restore_invalid_password_leaves_current_state(signed, fake_runners):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        change(db, owner, "keep this")
        db.commit()
        raw = await backup.create(db, "correct-backup-password")
        with pytest.raises(Exception):
            await backup.restore(db, raw, "incorrect-password")
        assert db.scalar(select(Memory)).data["content"] == "keep this"

async def test_database_failure_compensates_workspace(signed, fake_runners, monkeypatch):
    with SessionLocal() as db:
        raw = await backup.create(db, "test-backup-password")
        fake_runners["execution"] = {"files": {"current.txt": "bmV3"}}
        from contextlib import contextmanager
        @contextmanager
        def fail():
            raise RuntimeError("simulated database failure")
            yield
        monkeypatch.setattr(backup.engine, "begin", fail)
        with pytest.raises(RuntimeError):
            await backup.restore(db, raw, "test-backup-password")
        assert fake_runners["execution"]["files"] == {"current.txt": "bmV3"}
