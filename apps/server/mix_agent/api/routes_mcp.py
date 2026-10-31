import hashlib
import json
import secrets
import time
from datetime import timedelta

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import RedirectResponse
from sqlalchemy import delete, select

from mix_agent.api.common import (
    audit,
    context,
    own,
    public,
)
from mix_agent.api.schemas import (
    MCPInput,
    MCPInstallInput,
    MCPOAuthStartInput,
    MCPUninstallInput,
)
from mix_agent.auth.security import (
    read_secret,
    store_secret,
)
from mix_agent.db.models import (
    MCPAuthState,
    MCPConnection,
    Secret,
    Tool,
    now,
)
from mix_agent.db.session import get_db
from mix_agent.mcp import oauth as mcp_oauth
from mix_agent.mcp import registry as mcp_registry
from mix_agent.mcp.protocol import validate_schema as validate_mcp_schema
from mix_agent.tools.execute import runner_request

router = APIRouter(prefix="/api/v1")

# Backward-compatible re-exports for external callers (engine.py, scheduler, etc.)


@router.get("/mcp/connections")
def mcp_connections(ctx=Depends(context)):
    db, s = ctx
    return [public(r) for r in db.scalars(select(MCPConnection).where(MCPConnection.owner_id == s.owner_id))]


@router.get("/mcp/registry")
async def mcp_registry_search(q: str = "", cursor: str = "", limit: int = 30, ctx=Depends(context)):
    try:
        return await mcp_registry.search(q, cursor, limit)
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        raise HTTPException(502, "MCP Registryを取得できません")


@router.get("/mcp/registry/detail")
async def mcp_registry_detail(name: str, version: str = "latest", ctx=Depends(context)):
    try:
        return await mcp_registry.detail(name, version)
    except ValueError as error:
        raise HTTPException(422, str(error))
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        raise HTTPException(502, "MCP Registryの詳細を取得できません")


@router.post("/mcp/registry/install")
async def install_registry_mcp(body: MCPInstallInput, ctx=Depends(context)):
    db, s = ctx
    try:
        manifest = await mcp_registry.detail(body.registry_id, body.version)
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        raise HTTPException(502, "MCP Registryから導入情報を取得できません")
    candidate = manifest.get("selected")
    if not candidate:
        raise HTTPException(422, "対応するRemote、OCI、npm、PyPI配布がありません")
    existing = next(
        (
            row
            for row in db.scalars(select(MCPConnection).where(MCPConnection.owner_id == s.owner_id))
            if row.data.get("registry_id") == body.registry_id
        ),
        None,
    )
    if existing:
        raise HTTPException(409, "このMCPは導入済みです")
    row = MCPConnection(owner_id=s.owner_id, data={})
    db.add(row)
    db.flush()
    row_id = row.id
    secret_id = None
    if body.secrets:
        secret_id = store_secret(db, s.owner_id, json.dumps({"env": body.secrets}), "mcp")
    common = {
        "name": manifest["title"],
        "registry_id": manifest["registry_id"],
        "registry_version": manifest["version"],
        "manifest": manifest,
        "protocol_generation": "2026-07-28",
        "allow_legacy": True,
        "network_capability": {"mode": body.network_capability, "allowed_domains": body.allowed_domains},
        "configuration": body.configuration,
        "secret_id": secret_id,
        "enabled": True,
        "state": "installing",
        "authorization_required": False,
        "secret_required": False,
    }
    if candidate["kind"] == "remote":
        row.data = {
            **common,
            "transport": "http",
            "url": candidate["url"],
            "runtime": {"driver": "remote"},
            "state": "running",
        }
        try:
            audit(db, s.owner_id, "mcp.registry.install", row.id)
            db.commit()
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            db.rollback()
            raise HTTPException(422, "MCPを安全に導入できませんでした")
        return public(row)
    # Managed runtime installs can take minutes; persist the placeholder and
    # release the pooled DB connection before awaiting the manager instead of
    # pinning a session for the whole external call.
    row.data = {**common, "transport": "managed", "runtime": {}, "state": "installing"}
    try:
        db.commit()
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        db.rollback()
        raise HTTPException(422, "MCPを安全に導入できませんでした")
    db.close()
    try:
        runtime = await runner_request(
            "manager",
            "/v1/install",
            {
                "resource_id": row_id,
                "manifest": manifest,
                "network_capability": common["network_capability"],
            },
            timeout=360,
        )
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        failed = db.get(MCPConnection, row_id)
        if failed:
            db.delete(failed)
            if secret_id:
                secret = db.get(Secret, secret_id)
                if secret:
                    db.delete(secret)
            db.commit()
        raise HTTPException(422, "MCPを安全に導入できませんでした")
    row = db.get(MCPConnection, row_id)
    if not row:
        raise HTTPException(422, "MCPを安全に導入できませんでした")
    row.data = {**row.data, "runtime": runtime, "state": runtime.get("state", "running")}
    audit(db, s.owner_id, "mcp.registry.install", row.id)
    db.commit()
    return public(row)


@router.get("/mcp/oauth/client-metadata.json")
def mcp_oauth_client_metadata():
    return mcp_oauth.client_metadata()


@router.post("/mcp/connections/{key}/oauth/start")
async def start_mcp_oauth(key: str, body: MCPOAuthStartInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, MCPConnection, key, s.owner_id)
    if row.data.get("transport") != "http":
        raise HTTPException(422, "OAuthはRemote MCP専用です")
    try:
        discovery = await mcp_oauth.discover(row.data["url"])
        registration = await mcp_oauth.register(
            discovery, {"client_id": body.client_id, "client_secret": body.client_secret}
        )
        state, verifier = mcp_oauth.new_state()
        transient_secret = store_secret(
            db,
            s.owner_id,
            json.dumps({"verifier": verifier, "client_secret": registration.pop("client_secret", "")}),
            "mcp-oauth-state",
        )
        auth = MCPAuthState(
            owner_id=s.owner_id,
            state_hash=hashlib.sha256(state.encode()).hexdigest(),
            data={
                "connection_id": row.id,
                "expires_at": int(time.time()) + 600,
                "discovery": discovery,
                "registration": registration,
                "secret_id": transient_secret,
            },
        )
        db.add(auth)
        row.data = {**row.data, "state": "authorizing", "authorization_required": True}
        db.commit()
        return {
            "authorization_url": mcp_oauth.authorization(
                discovery, registration, state, verifier, body.scopes
            ),
            "registration_method": registration["method"],
        }
    except ValueError as error:
        raise HTTPException(422, str(error))


@router.get("/mcp/oauth/callback")
async def mcp_oauth_callback(
    state: str = "", code: str = "", iss: str = "", error: str = "", db=Depends(get_db)
):
    digest = hashlib.sha256(state.encode()).hexdigest()
    # Authorization states are single-use and expire after ten minutes; purge
    # abandoned ones so this unauthenticated endpoint stays bounded.
    db.execute(delete(MCPAuthState).where(MCPAuthState.created_at < now() - timedelta(hours=1)))
    db.commit()
    auth = db.scalar(select(MCPAuthState).where(MCPAuthState.state_hash == digest))
    if (
        not auth
        or not secrets.compare_digest(auth.state_hash, digest)
        or auth.data.get("expires_at", 0) < int(time.time())
        or error
        or not code
    ):
        return RedirectResponse("/settings/mcp?oauth=failed", status_code=303)
    row = db.get(MCPConnection, auth.data["connection_id"])
    if not row or row.owner_id != auth.owner_id:
        return RedirectResponse("/settings/mcp?oauth=failed", status_code=303)
    transient = json.loads(read_secret(db, auth.data.get("secret_id")) or "{}")
    registration = {**auth.data["registration"], "client_secret": transient.get("client_secret", "")}
    try:
        token = await mcp_oauth.exchange(
            auth.data["discovery"], registration, code, transient["verifier"], iss
        )
        credential = {
            "oauth": token,
            "oauth_registration": registration,
            "headers": {"Authorization": "Bearer " + token["access_token"]},
        }
        secret_id = store_secret(db, row.owner_id, json.dumps(credential), "mcp")
        row.data = {
            **row.data,
            "secret_id": secret_id,
            "state": "running",
            "authorization_required": False,
            "oauth_registration": registration["method"],
        }
        db.delete(auth)
        audit(db, row.owner_id, "mcp.oauth.authorized", row.id)
        db.commit()
        return RedirectResponse("/settings/mcp?oauth=success", status_code=303)
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        return RedirectResponse("/settings/mcp?oauth=failed", status_code=303)


@router.post("/mcp/connections")
def create_mcp(body: MCPInput, ctx=Depends(context)):
    db, s = ctx
    data = body.model_dump(exclude={"credentials"})
    if body.credentials:
        data["secret_id"] = store_secret(db, s.owner_id, json.dumps(body.credentials), "mcp")
    row = MCPConnection(owner_id=s.owner_id, data=data)
    db.add(row)
    db.flush()
    audit(db, s.owner_id, "mcp.create", row.id)
    db.commit()
    return public(row)


@router.patch("/mcp/connections/{key}")
def update_mcp(key: str, body: MCPInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, MCPConnection, key, s.owner_id)
    data = body.model_dump(exclude={"credentials"})
    data["secret_id"] = row.data.get("secret_id")
    if body.credentials is not None:
        data["secret_id"] = store_secret(db, s.owner_id, json.dumps(body.credentials), "mcp")
    row.data = data
    # Connection edits invalidate all previously discovered definitions and grants.
    for tool in db.scalars(select(Tool).where(Tool.owner_id == s.owner_id)):
        if tool.data.get("source_ref") == key:
            tool.data = {**tool.data, "enabled": False}
    db.commit()
    return public(row)


@router.post("/mcp/connections/{key}/{action}")
async def mcp_action(key: str, action: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, MCPConnection, key, s.owner_id)
    if action == "disconnect":
        row.data = {**row.data, "enabled": False}
        for tool in db.scalars(select(Tool).where(Tool.owner_id == s.owner_id)):
            if tool.data.get("source_ref") == key:
                tool.data = {**tool.data, "enabled": False}
        db.commit()
        return {"ok": True}
    if action not in ("sync", "test", "install"):
        raise HTTPException(404)
    credentials = json.loads(read_secret(db, row.data.get("secret_id")) or "{}")
    connection = {**row.data, "credentials": credentials}
    driver = row.data.get("runtime", {}).get("driver")
    # Sync/test/install can take minutes; release the pooled connection while
    # awaiting the runner instead of pinning a session for the whole call.
    db.close()
    if driver in ("oci", "npm", "pypi"):
        if action == "install":
            raise HTTPException(409, "Registry MCPは導入済みです")
        result = await runner_request(
            "manager",
            f"/v1/discover/{key}",
            {
                "credentials": credentials,
                "arguments": connection.get("runtime_arguments", []),
            },
            timeout=360,
        )
    else:
        result = await runner_request(
            "mcp", "/" + ("install" if action == "install" else "discover"), {"connection": connection}
        )
    if action == "sync":
        row = db.get(MCPConnection, key)
        if not row:
            raise HTTPException(404)
        prefix = key.replace("-", "")[:8]
        seen = set()
        for t in result["tools"]:
            validate_mcp_schema(t["inputSchema"])
            validate_mcp_schema(t.get("outputSchema", {}))
            tool_id = hashlib.sha256((key + ":" + t["name"]).encode()).hexdigest()[:36]
            seen.add(tool_id)
            data = {
                "model_name": ("mcp_" + prefix + "_" + t["name"])[:64],
                "description": t.get("description", "MCP tool"),
                "input_schema": t["inputSchema"],
                "output_schema": t.get("outputSchema", {"type": "object"}),
                "source": "mcp",
                "source_ref": key,
                "remote_name": t["name"],
                "executor_ref": "mcp",
                "default_permission": "ask",
                "risk": "external",
                "enabled": True,
                "version": hashlib.sha256(json.dumps(t, sort_keys=True).encode()).hexdigest(),
            }
            old = db.get(Tool, tool_id)
            if old:
                old.data = data
            else:
                db.add(Tool(id=tool_id, owner_id=s.owner_id, data=data))
        for old in db.scalars(select(Tool).where(Tool.owner_id == s.owner_id)):
            if old.data.get("source_ref") == key and old.id not in seen:
                old.data = {**old.data, "enabled": False}
        row.data = {**row.data, "enabled": True}
        audit(db, s.owner_id, "mcp.sync", key)
        db.commit()
    return result


@router.post("/mcp/connections/{key}/runtime/{action}")
async def mcp_runtime_action(key: str, action: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, MCPConnection, key, s.owner_id)
    if action not in ("start", "stop", "restart", "status"):
        raise HTTPException(404)
    if row.data.get("runtime", {}).get("driver") not in ("oci", "npm", "pypi"):
        raise HTTPException(422, "ローカルMCP専用の操作です")
    result = await runner_request("manager", f"/v1/runtime/{key}/{action}", {})
    if action != "status":
        state = "stopped" if action == "stop" else "running"
        row.data = {**row.data, "state": state, "enabled": state == "running"}
        audit(db, s.owner_id, "mcp.runtime." + action, key)
        db.commit()
    return result


@router.post("/mcp/connections/{key}/update")
async def update_registry_mcp(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, MCPConnection, key, s.owner_id)
    if not row.data.get("registry_id"):
        raise HTTPException(422, "Registry MCPではありません")
    new_manifest = await mcp_registry.detail(row.data["registry_id"], "latest")
    if new_manifest["version"] == row.data.get("registry_version"):
        return {"ok": True, "updated": False, "version": new_manifest["version"]}
    credentials = json.loads(read_secret(db, row.data.get("secret_id")) or "{}")
    driver = row.data.get("runtime", {}).get("driver")
    if driver == "remote":
        row.data = {**row.data, "manifest": new_manifest, "registry_version": new_manifest["version"]}
    else:
        runtime = await runner_request(
            "manager",
            f"/v1/update/{key}",
            {
                "old_manifest": row.data["manifest"],
                "new_manifest": new_manifest,
                "network_capability": row.data.get("network_capability", {}),
                "health": {"credentials": credentials, "arguments": row.data.get("runtime_arguments", [])},
            },
            timeout=600,
        )
        row.data = {
            **row.data,
            "manifest": new_manifest,
            "registry_version": new_manifest["version"],
            "runtime": runtime,
            "state": "running",
        }
    audit(db, s.owner_id, "mcp.registry.update", key)
    db.commit()
    return public(row)


@router.post("/mcp/connections/{key}/uninstall")
async def uninstall_mcp(key: str, body: MCPUninstallInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, MCPConnection, key, s.owner_id)
    runtime = row.data.get("runtime", {})
    if body.delete_volume:
        expected = runtime.get("volume", "")
        if not expected or body.confirm_volume != expected:
            raise HTTPException(422, "完全削除するvolume名が一致しません")
    if runtime.get("driver") in ("oci", "npm", "pypi"):
        await runner_request("manager", f"/v1/runtime/{key}/uninstall", {"delete_volume": body.delete_volume})
    for tool in list(db.scalars(select(Tool).where(Tool.owner_id == s.owner_id))):
        if tool.data.get("source_ref") == key:
            db.delete(tool)
    audit(
        db, s.owner_id, "mcp.uninstall.complete" if body.delete_volume else "mcp.uninstall.keep-volume", key
    )
    if body.delete_volume or runtime.get("driver") == "remote":
        db.delete(row)
    else:
        row.data = {**row.data, "state": "uninstalled_data_retained", "enabled": False}
    db.commit()
    return {"ok": True, "volume_deleted": body.delete_volume}


@router.post("/mcp/connections/{key}/reinstall")
async def reinstall_mcp(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, MCPConnection, key, s.owner_id)
    if row.data.get("state") != "uninstalled_data_retained":
        raise HTTPException(409, "保持データから再導入できる状態ではありません")
    runtime = await runner_request(
        "manager",
        "/v1/install",
        {
            "resource_id": row.id,
            "manifest": row.data["manifest"],
            "network_capability": row.data.get("network_capability", {}),
        },
        timeout=360,
    )
    row.data = {**row.data, "runtime": runtime, "state": "running", "enabled": True}
    audit(db, s.owner_id, "mcp.reinstall", key)
    db.commit()
    return public(row)
