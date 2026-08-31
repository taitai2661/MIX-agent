"""Encrypted, bounded backups. Validate first, retain rollback copies until commit."""

import asyncio
import base64
import hashlib
import json
import os
from datetime import datetime
from pathlib import Path
from uuid import uuid4
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.scrypt import Scrypt
from sqlalchemy import select, DateTime
from mix_agent import config
from mix_agent.db.models import Base, Run
from mix_agent.db.session import engine
from mix_agent.tools.execute import runner_request

ACTIVE = False
LOCK = asyncio.Lock()
MAGIC = b"MIXBACKUP1\n"
MAX_SIZE = 256 * 1024 * 1024


def seal(payload, password):
    salt, nonce = os.urandom(16), os.urandom(12)
    key = Scrypt(salt=salt, length=32, n=2**15, r=8, p=1).derive(password.encode())
    return MAGIC + salt + nonce + AESGCM(key).encrypt(nonce, payload, MAGIC)


def unseal(raw, password):
    if len(raw) > MAX_SIZE or not raw.startswith(MAGIC):
        raise ValueError("Invalid backup file")
    offset = len(MAGIC)
    salt, nonce = raw[offset : offset + 16], raw[offset + 16 : offset + 28]
    key = Scrypt(salt=salt, length=32, n=2**15, r=8, p=1).derive(password.encode())
    return AESGCM(key).decrypt(nonce, raw[offset + 28 :], MAGIC)


def check_idle(db):
    if db.scalar(select(Run.id).where(Run.status.in_(["queued", "running", "waiting_approval"]))):
        raise ValueError("すべての実行を停止してからバックアップしてください")


async def create(db, password):
    check_idle(db)
    workspace = await runner_request("execution", "/snapshot", {})
    mcp = await runner_request("mcp", "/snapshot", {})
    tables = {}
    for table in Base.metadata.sorted_tables:
        tables[table.name] = [dict(row) for row in db.execute(select(table)).mappings()]
    files = {}
    for file in config.ARTIFACTS.iterdir():
        if file.is_file() and not file.is_symlink():
            files[file.name] = base64.b64encode(file.read_bytes()).decode()
    payload = json.dumps(
        {
            "version": 1,
            "schema": "0001",
            "tables": tables,
            "artifacts": files,
            "workspace": workspace,
            "mcp_shared": mcp,
            "master_key": base64.b64encode((config.KEYS / "master.key").read_bytes()).decode(),
        },
        default=lambda x: x.isoformat() if isinstance(x, datetime) else str(x),
    ).encode()
    if len(payload) > MAX_SIZE - 100:
        raise ValueError("初期版のバックアップ上限は256MBです")
    return seal(payload, password)


def validate(raw, password):
    payload = json.loads(unseal(raw, password))
    if payload.get("version") != 1 or payload.get("schema") != "0001":
        raise ValueError("Unsupported backup version")
    if set(payload["tables"]) != {t.name for t in Base.metadata.sorted_tables}:
        raise ValueError("Schema mismatch")
    key = base64.b64decode(payload["master_key"], validate=True)
    if len(key) != 32:
        raise ValueError("Invalid recovery key")
    from uuid import UUID

    for name, content in payload["artifacts"].items():
        UUID(name)
        base64.b64decode(content, validate=True)
    for row in payload["tables"]["artifacts"]:
        raw_file = base64.b64decode(payload["artifacts"][row["id"]])
        if hashlib.sha256(raw_file).hexdigest() != row["data"]["sha256"]:
            raise ValueError("Artifact checksum mismatch")
    if len(payload["tables"]["users"]) != 1:
        raise ValueError("Backup must contain exactly one administrator")
    if any(r["status"] in ("running", "queued", "waiting_approval") for r in payload["tables"]["runs"]):
        raise ValueError("Backup contains active runs")
    return payload


async def restore(db, raw, password, recovering=False):
    check_idle(db)
    payload = validate(raw, password)
    await runner_request("execution", "/validate-snapshot", payload["workspace"])
    await runner_request("mcp", "/validate-snapshot", payload["mcp_shared"])
    journal = config.DATA / "restore-journal.json"
    recovery_key = config.KEYS / "restore-passphrase"
    if not recovering:
        rollback_blob = await create(db, password)
        (config.DATA / "restore-rollback.mix").write_bytes(rollback_blob)
        recovery_key.write_text(password)
        recovery_key.chmod(0o600)
        journal.write_text(json.dumps({"phase": "restoring"}))
    old_workspace = await runner_request("execution", "/snapshot", {})
    old_mcp = await runner_request("mcp", "/snapshot", {})
    old_key = (config.KEYS / "master.key").read_bytes()
    stage = config.DATA / ("restore-" + str(uuid4()))
    stage.mkdir()
    for name, content in payload["artifacts"].items():
        (stage / name).write_bytes(base64.b64decode(content))
    previous = config.DATA / ("rollback-" + str(uuid4()))
    swapped = False
    moved_original = False
    try:
        await runner_request("execution", "/restore-snapshot", payload["workspace"])
        await runner_request("mcp", "/restore-snapshot", payload["mcp_shared"])
        # One transaction for all database tables. The original files remain available until commit.
        db.rollback()
        with engine.begin() as conn:
            for table in reversed(Base.metadata.sorted_tables):
                conn.execute(table.delete())
            for table in Base.metadata.sorted_tables:
                rows = payload["tables"][table.name]
                if table.name == "sessions":
                    rows = []  # Require login with restored administrator credentials.
                for row in rows:
                    for column in table.columns:
                        if isinstance(column.type, DateTime) and row.get(column.name):
                            row[column.name] = datetime.fromisoformat(row[column.name])
                if rows:
                    conn.execute(table.insert(), rows)
            config.ARTIFACTS.rename(previous)
            moved_original = True
            stage.rename(config.ARTIFACTS)
            swapped = True
            (config.KEYS / "master.key").write_bytes(base64.b64decode(payload["master_key"]))
    except BaseException:
        (config.KEYS / "master.key").write_bytes(old_key)
        if swapped:
            config.ARTIFACTS.rename(stage)
        if moved_original:
            previous.rename(config.ARTIFACTS)
        # Keep the encrypted rollback archive for recovery even if a runner is unavailable.
        try:
            await runner_request("execution", "/restore-snapshot", old_workspace)
            await runner_request("mcp", "/restore-snapshot", old_mcp)
            journal.unlink(missing_ok=True)
            recovery_key.unlink(missing_ok=True)
        finally:
            raise
    # Keep previous artifacts for operator recovery. Do not erase automatically.
    setting = payload["tables"]["app_settings"]
    domains = setting[0]["data"].get("allowed_domains", []) if setting else []
    policy = config.DATA / "egress"
    policy.mkdir(exist_ok=True)
    (policy / "policy.json").write_text(json.dumps({"allowed_domains": domains}))
    journal.unlink(missing_ok=True)
    recovery_key.unlink(missing_ok=True)
    return {"ok": True, "relogin_required": True}


async def recover_interrupted():
    global ACTIVE
    if not (config.DATA / "restore-journal.json").exists():
        return
    ACTIVE = True
    from mix_agent.db.session import SessionLocal

    with SessionLocal() as db:
        await restore(
            db,
            (config.DATA / "restore-rollback.mix").read_bytes(),
            (config.KEYS / "restore-passphrase").read_text(),
            recovering=True,
        )
    ACTIVE = False
