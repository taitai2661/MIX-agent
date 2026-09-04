"""Encrypted, bounded backups. Validate first, retain rollback copies until commit."""

import asyncio
import base64
import hashlib
import json
import os
from datetime import datetime
from uuid import uuid4

from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.scrypt import Scrypt
from sqlalchemy import DateTime, select

from mix_agent import config
from mix_agent.db.models import Base, MCPConnection, Run
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
    managed_ids = [row.id for row in db.scalars(select(MCPConnection)) if row.data.get("runtime", {}).get("driver") in ("oci", "npm", "pypi")]
    managed_mcp = await runner_request("manager", "/v1/snapshot", {"resource_ids": managed_ids}) if managed_ids else {"volumes": {}}
    tables = {}
    for table in Base.metadata.sorted_tables:
        rows = [dict(row) for row in db.execute(select(table)).mappings()]
        if table.name in ("secrets", "mcp_auth_states", "sessions"):
            rows = []
        for row in rows:
            data = row.get("data")
            if isinstance(data, dict):
                had_secret = any(key.endswith("secret_id") and value for key, value in data.items())
                row["data"] = {key: value for key, value in data.items() if not key.endswith("secret_id")}
                if table.name == "mcp_connections" and had_secret:
                    row["data"]["secret_required"] = True
                    row["data"]["state"] = "needs_configuration"
                    row["data"]["enabled"] = False
        tables[table.name] = rows
    files = {}
    for file in config.ARTIFACTS.iterdir():
        if file.is_file() and not file.is_symlink():
            files[file.name] = base64.b64encode(file.read_bytes()).decode()
    payload = json.dumps(
        {
            "version": 2,
            "schema": "0010",
            "tables": tables,
            "artifacts": files,
            "workspace": workspace,
            "mcp_shared": mcp,
            "mcp_volumes": managed_mcp,
            "secrets_included": False,
        },
        default=lambda x: x.isoformat() if isinstance(x, datetime) else str(x),
    ).encode()
    if len(payload) > MAX_SIZE - 100:
        raise ValueError("初期版のバックアップ上限は256MBです")
    return seal(payload, password)


def validate(raw, password):
    payload = json.loads(unseal(raw, password))
    if payload.get("version") not in (1, 2) or payload.get("schema") not in ("0001", "0002", "0010"):
        raise ValueError("Unsupported backup version")
    expected_tables = {t.name for t in Base.metadata.sorted_tables}
    backup_tables = set(payload["tables"])
    if payload.get("schema") == "0001" and backup_tables == expected_tables - {"login_events"}:
        payload["tables"]["login_events"] = []
    for missing in expected_tables - set(payload["tables"]):
        if missing in ("mcp_auth_states",):
            payload["tables"][missing] = []
    if set(payload["tables"]) != expected_tables:
        raise ValueError("Schema mismatch")
    # Secrets and transient OAuth state are never accepted from a backup,
    # including legacy archives that contained encrypted Secret rows.
    payload["tables"]["secrets"] = []
    payload["tables"]["mcp_auth_states"] = []
    payload["tables"]["sessions"] = []
    for row in payload["tables"]["mcp_connections"]:
        data = row["data"]
        had_secret = any(key.endswith("secret_id") and value for key, value in data.items())
        row["data"] = {key: value for key, value in data.items() if not key.endswith("secret_id")}
        if had_secret or data.get("secret_required"):
            row["data"].update(secret_required=True, state="needs_configuration", enabled=False)
        elif data.get("oauth_registration") or data.get("authorization_required"):
            row["data"].update(authorization_required=True, state="needs_authorization", enabled=False)
    payload.setdefault("mcp_volumes", {"volumes": {}})
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
    await runner_request("manager", "/v1/validate-snapshot", payload["mcp_volumes"])
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
    managed_ids = [row.id for row in db.scalars(select(MCPConnection)) if row.data.get("runtime", {}).get("driver") in ("oci", "npm", "pypi")]
    old_managed = await runner_request("manager", "/v1/snapshot", {"resource_ids": managed_ids}) if managed_ids else {"volumes": {}}
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
        await runner_request("manager", "/v1/restore-snapshot", payload["mcp_volumes"])
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
    except BaseException:
        if swapped:
            config.ARTIFACTS.rename(stage)
        if moved_original:
            previous.rename(config.ARTIFACTS)
        # Keep the encrypted rollback archive for recovery even if a runner is unavailable.
        try:
            await runner_request("execution", "/restore-snapshot", old_workspace)
            await runner_request("mcp", "/restore-snapshot", old_mcp)
            await runner_request("manager", "/v1/restore-snapshot", old_managed)
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
