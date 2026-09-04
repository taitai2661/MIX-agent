"""Small, policy-constrained Docker lifecycle manager for local MCP workloads."""

from __future__ import annotations

import asyncio
import base64
import itertools
import json
import os
import re
import secrets
import uuid
from abc import ABC, abstractmethod
from pathlib import Path

import httpx
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse

app = FastAPI(docs_url=None, redoc_url=None)
LABEL = "io.mix-agent.mcp"
NAME_RE = re.compile(r"^[a-f0-9-]{1,36}$")
DOMAIN_RE = re.compile(r"^(?=.{1,253}$)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)*[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$")
RUNTIME_IMAGE = os.getenv("MCP_RUNTIME_IMAGE", "mix-agent-mcp-runtime:local")
MANAGER_CONTAINER = os.getenv("HOSTNAME", "mcp-manager")
EGRESS_CONTAINER = os.getenv("MCP_EGRESS_CONTAINER", "mix-agent-egress-proxy-1")
POLICY = Path(os.getenv("POLICY_FILE", "/policy/mcp-policy.json"))
LOCKS: dict[str, asyncio.Lock] = {}
OCI_CHANNELS: dict[str, tuple[object, asyncio.Lock, str]] = {}
RPC_IDS = itertools.count(1)


@app.middleware("http")
async def authenticate(request: Request, call_next):
    token = Path(os.getenv("TOKEN_FILE", "/tokens/manager.token")).read_text().strip()
    if not secrets.compare_digest(request.headers.get("authorization", ""), "Bearer " + token):
        return JSONResponse({"detail": "Unauthorized"}, 401)
    return await call_next(request)


class ContainerRuntimeDriver(ABC):
    @abstractmethod
    async def install(self, resource_id: str, manifest: dict, capability: dict) -> dict: ...

    @abstractmethod
    async def action(self, resource_id: str, action: str, delete_volume: bool = False) -> dict: ...


class DockerAPI:
    def __init__(self):
        self.client = httpx.AsyncClient(
            transport=httpx.AsyncHTTPTransport(uds=os.getenv("DOCKER_SOCKET", "/var/run/docker.sock")),
            base_url="http://docker/v1.47",
            timeout=120,
            trust_env=False,
        )

    async def call(self, method: str, path: str, *, body=None, ok=(200, 201, 204, 304)) -> dict:
        response = await self.client.request(method, path, json=body)
        if response.status_code not in ok:
            message = "Docker operation failed"
            try:
                message = response.json().get("message", message)
            except Exception:
                pass
            raise RuntimeError(message)
        if not response.content:
            return {}
        return response.json()

    async def bytes(self, method: str, path: str, *, content: bytes | None = None, headers: dict | None = None, ok=(200,)) -> bytes:
        response = await self.client.request(method, path, content=content, headers=headers)
        if response.status_code not in ok:
            raise RuntimeError("Docker archive operation failed")
        return response.content


def _names(resource_id: str) -> tuple[str, str, str]:
    if not NAME_RE.fullmatch(resource_id):
        raise ValueError("Invalid MCP resource id")
    short = resource_id.replace("-", "")
    return f"mix-mcp-{short}", f"mix-mcp-{short}-net", f"mix-mcp-{short}-data"


def _capability(value: dict) -> dict:
    mode = value.get("mode", "none")
    if mode not in ("none", "restricted", "public_web"):
        raise ValueError("Invalid network capability")
    domains = []
    for raw in value.get("allowed_domains", []):
        host = str(raw).lower().rstrip(".")
        if not DOMAIN_RE.fullmatch(host):
            raise ValueError("Invalid allowed domain")
        domains.append(host)
    if mode == "restricted" and not domains:
        raise ValueError("Restricted network capability needs allowed domains")
    return {"mode": mode, "allowed_domains": sorted(set(domains))}


def _load_policy() -> dict:
    try:
        value = json.loads(POLICY.read_text())
        return value if isinstance(value, dict) else {}
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def _save_policy(value: dict) -> None:
    POLICY.parent.mkdir(parents=True, exist_ok=True)
    temporary = POLICY.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, sort_keys=True))
    temporary.replace(POLICY)


class DockerRuntimeDriver(ContainerRuntimeDriver):
    def __init__(self):
        self.api = DockerAPI()

    async def _connect(self, network: str, container: str, aliases: list[str] | None = None):
        try:
            await self.api.call("POST", f"/networks/{network}/connect", body={"Container": container, "EndpointConfig": {"Aliases": aliases or []}})
        except RuntimeError as error:
            if "already exists" not in str(error).lower():
                raise

    async def prepare_volume(self, volume: str) -> None:
        helper = "mix-mcp-volume-init-" + uuid.uuid4().hex
        body = {
            "Image": RUNTIME_IMAGE,
            "Cmd": ["python", "-c", "import os,pathlib;[(pathlib.Path('/data')/p).mkdir(exist_ok=True) for p in ('.packages','workspace')];[os.chown('/data/'+p,1000,1000) for p in ('.packages','workspace')]"],
            "User": "0:0", "Labels": {LABEL: "volume-init"},
            "HostConfig": {"NetworkMode": "none", "ReadonlyRootfs": True, "CapDrop": ["ALL"], "CapAdd": ["CHOWN"],
                           "SecurityOpt": ["no-new-privileges:true"], "Mounts": [{"Type": "volume", "Source": volume, "Target": "/data", "ReadOnly": False}]},
        }
        await self.api.call("POST", f"/containers/create?name={helper}", body=body)
        try:
            await self.api.call("POST", f"/containers/{helper}/start", ok=(204,))
            result = await self.api.call("POST", f"/containers/{helper}/wait?condition=not-running")
            if result.get("StatusCode"):
                raise RuntimeError("MCP volume initialization failed")
        finally:
            await self.api.call("DELETE", f"/containers/{helper}?force=true", ok=(204, 404))

    async def install(self, resource_id: str, manifest: dict, capability: dict) -> dict:
        container, network, volume = _names(resource_id)
        candidate = manifest.get("selected") or {}
        kind = candidate.get("kind")
        if kind not in ("oci", "npm", "pypi"):
            raise ValueError("Unsupported local MCP package")
        capability = _capability(capability)
        await self.api.call("POST", "/volumes/create", body={"Name": volume, "Labels": {LABEL: resource_id}})
        await self.prepare_volume(volume)
        try:
            await self.api.call("POST", "/networks/create", body={"Name": network, "Internal": True, "Labels": {LABEL: resource_id}})
        except RuntimeError as error:
            if "already exists" not in str(error).lower():
                raise
        image = candidate["identifier"] if kind == "oci" else RUNTIME_IMAGE
        env = [
            f"MCP_PACKAGE_KIND={kind}",
            f"MCP_PACKAGE_IDENTIFIER={candidate['identifier']}",
            f"MCP_PACKAGE_VERSION={candidate.get('version', manifest.get('version', ''))}",
            "HOME=/data/workspace",
            "TMPDIR=/tmp",
            "PYTHONPATH=/app",
        ]
        if capability["mode"] != "none" or kind in ("npm", "pypi"):
            env.extend([
                "HTTP_PROXY=http://egress-proxy:3128",
                "HTTPS_PROXY=http://egress-proxy:3128",
                "http_proxy=http://egress-proxy:3128",
                "https_proxy=http://egress-proxy:3128",
            ])
        config = {
            "Image": image,
            "Env": env,
            "Labels": {LABEL: resource_id, f"{LABEL}.driver": kind},
            "User": "1000:1000",
            "WorkingDir": "/data",
            "HostConfig": {
                "ReadonlyRootfs": True,
                "CapDrop": ["ALL"],
                "SecurityOpt": ["no-new-privileges:true"],
                "NetworkMode": network,
                "Mounts": [
                    {"Type": "volume", "Source": volume, "Target": "/data", "ReadOnly": False},
                    {"Type": "tmpfs", "Target": "/tmp", "TmpfsOptions": {"SizeBytes": 268435456, "Mode": 0o1777}},
                    {"Type": "tmpfs", "Target": "/run/secrets/mcp", "TmpfsOptions": {"SizeBytes": 1048576, "Mode": 0o700}},
                ],
                "PidsLimit": 128,
                "Memory": 536870912,
                "NanoCpus": 1_000_000_000,
                "AutoRemove": False,
            },
            "OpenStdin": kind == "oci",
            "StdinOnce": False,
            "AttachStdin": kind == "oci",
            "AttachStdout": kind == "oci",
            "AttachStderr": kind == "oci",
        }
        if kind in ("npm", "pypi"):
            config["ExposedPorts"] = {"8004/tcp": {}}
        try:
            await self.api.call("POST", f"/containers/create?name={container}", body=config)
        except RuntimeError as error:
            if "already in use" not in str(error).lower():
                raise
        await self._connect(network, MANAGER_CONTAINER, ["mcp-manager"])
        needs_install_egress = kind in ("npm", "pypi")
        if capability["mode"] != "none" or needs_install_egress:
            await self._connect(network, EGRESS_CONTAINER, ["egress-proxy"])
        inspect = await self.api.call("GET", f"/containers/{container}/json")
        ip = inspect.get("NetworkSettings", {}).get("Networks", {}).get(network, {}).get("IPAddress", "")
        policy = _load_policy()
        policies = policy.setdefault("source_policies", {})
        if ip:
            policies[ip] = ({"mode": "restricted", "allowed_domains": ["registry.npmjs.org"]} if kind == "npm" else {"mode": "restricted", "allowed_domains": ["pypi.org", "files.pythonhosted.org"]}) if needs_install_egress else capability
        policy.setdefault("resource_policies", {})[resource_id] = {"ip": ip, "runtime": capability, "installing": needs_install_egress, "driver": kind}
        _save_policy(policy)
        await self.api.call("POST", f"/containers/{container}/start", ok=(204, 304))
        return {"container": container, "network": network, "volume": volume, "driver": kind, "state": "running"}

    async def action(self, resource_id: str, action: str, delete_volume: bool = False) -> dict:
        container, network, volume = _names(resource_id)
        if action in ("start", "stop", "restart"):
            await self.api.call("POST", f"/containers/{container}/{action}?t=20", ok=(204, 304))
        elif action == "status":
            value = await self.api.call("GET", f"/containers/{container}/json")
            return {"state": value.get("State", {}), "container": container, "network": network, "volume": volume}
        elif action == "uninstall":
            try:
                await self.api.call("POST", f"/containers/{container}/stop?t=20", ok=(204, 304))
            except RuntimeError:
                pass
            await self.api.call("DELETE", f"/containers/{container}?force=false&v=false", ok=(204, 404))
            for attached in (MANAGER_CONTAINER, EGRESS_CONTAINER):
                try:
                    await self.api.call("POST", f"/networks/{network}/disconnect", body={"Container": attached, "Force": True})
                except RuntimeError:
                    pass
            await self.api.call("DELETE", f"/networks/{network}", ok=(204, 404))
            policy = _load_policy()
            resource = policy.get("resource_policies", {}).pop(resource_id, {})
            if resource.get("ip"):
                policy.get("source_policies", {}).pop(resource["ip"], None)
            _save_policy(policy)
            channel = OCI_CHANNELS.pop(resource_id, None)
            if channel:
                await channel[0].close()
            if delete_volume:
                await self.api.call("DELETE", f"/volumes/{volume}", ok=(204, 404))
        else:
            raise ValueError("Unknown runtime action")
        return {"ok": True, "container": container, "network": network, "volume": volume}

    async def copy_volume(self, source: str, destination: str, *, clear: bool = False) -> None:
        helper = "mix-mcp-copy-" + uuid.uuid4().hex
        script = (
            "import pathlib,shutil; src=pathlib.Path('/source'); dst=pathlib.Path('/destination'); "
            + ("[shutil.rmtree(p) if p.is_dir() else p.unlink() for p in list(dst.iterdir())]; " if clear else "")
            + "[shutil.copytree(p,dst/p.name,symlinks=False) if p.is_dir() else shutil.copy2(p,dst/p.name) for p in src.iterdir()]"
        )
        body = {
            "Image": RUNTIME_IMAGE, "Cmd": ["python", "-c", script], "User": "0:0",
            "Labels": {LABEL: "volume-copy"},
            "HostConfig": {"NetworkMode": "none", "ReadonlyRootfs": True, "CapDrop": ["ALL"], "SecurityOpt": ["no-new-privileges:true"],
                           "Mounts": [{"Type": "volume", "Source": source, "Target": "/source", "ReadOnly": True},
                                      {"Type": "volume", "Source": destination, "Target": "/destination", "ReadOnly": False}]},
        }
        await self.api.call("POST", f"/containers/create?name={helper}", body=body)
        try:
            await self.api.call("POST", f"/containers/{helper}/start", ok=(204,))
            result = await self.api.call("POST", f"/containers/{helper}/wait?condition=not-running")
            if result.get("StatusCode"):
                raise RuntimeError("Volume snapshot failed")
        finally:
            await self.api.call("DELETE", f"/containers/{helper}?force=true", ok=(204, 404))

    async def update(self, resource_id: str, old_manifest: dict, new_manifest: dict, capability: dict, health: dict) -> dict:
        _, _, volume = _names(resource_id)
        backup = volume + "-backup-" + uuid.uuid4().hex[:12]
        await self.api.call("POST", "/volumes/create", body={"Name": backup, "Labels": {LABEL: resource_id, f"{LABEL}.backup": "true"}})
        await self.copy_volume(volume, backup)
        await self.action(resource_id, "stop")
        await self.action(resource_id, "uninstall", False)
        try:
            runtime = await self.install(resource_id, new_manifest, capability)
            candidate = new_manifest.get("selected") or {}
            if candidate.get("kind") in ("npm", "pypi"):
                await _proxy(resource_id, "discover", health)
            await self.api.call("DELETE", f"/volumes/{backup}", ok=(204, 404))
            return {**runtime, "updated": True}
        except Exception:
            try:
                await self.action(resource_id, "uninstall", False)
            except Exception:
                pass
            await self.copy_volume(backup, volume, clear=True)
            await self.install(resource_id, old_manifest, capability)
            raise RuntimeError("MCP update failed; previous version was restored")


DRIVER: ContainerRuntimeDriver = DockerRuntimeDriver()


async def _volume_archive(resource_id: str) -> bytes:
    if not isinstance(DRIVER, DockerRuntimeDriver):
        raise ValueError("Runtime does not support snapshots")
    _, _, volume = _names(resource_id)
    helper = "mix-mcp-snapshot-" + uuid.uuid4().hex
    body = {
        "Image": RUNTIME_IMAGE, "Cmd": ["python", "-c", "import time;time.sleep(300)"], "User": "0:0",
        "Labels": {LABEL: "snapshot"},
        "HostConfig": {"NetworkMode": "none", "ReadonlyRootfs": True, "CapDrop": ["ALL"], "SecurityOpt": ["no-new-privileges:true"],
                       "Mounts": [{"Type": "volume", "Source": volume, "Target": "/data", "ReadOnly": False}]},
    }
    await DRIVER.api.call("POST", f"/containers/create?name={helper}", body=body)
    try:
        await DRIVER.api.call("POST", f"/containers/{helper}/start", ok=(204,))
        return await DRIVER.api.bytes("GET", f"/containers/{helper}/archive?path=/data")
    finally:
        await DRIVER.api.call("DELETE", f"/containers/{helper}?force=true", ok=(204, 404))


async def _restore_archive(resource_id: str, archive: bytes) -> None:
    if not isinstance(DRIVER, DockerRuntimeDriver):
        raise ValueError("Runtime does not support snapshots")
    _, _, volume = _names(resource_id)
    await DRIVER.api.call("POST", "/volumes/create", body={"Name": volume, "Labels": {LABEL: resource_id}})
    helper = "mix-mcp-restore-" + uuid.uuid4().hex
    body = {
        "Image": RUNTIME_IMAGE, "Cmd": ["python", "-c", "import time;time.sleep(300)"], "User": "0:0",
        "Labels": {LABEL: "restore"},
        "HostConfig": {"NetworkMode": "none", "ReadonlyRootfs": True, "CapDrop": ["ALL"], "SecurityOpt": ["no-new-privileges:true"],
                       "Mounts": [{"Type": "volume", "Source": volume, "Target": "/data", "ReadOnly": False}]},
    }
    await DRIVER.api.call("POST", f"/containers/create?name={helper}", body=body)
    try:
        await DRIVER.api.call("POST", f"/containers/{helper}/start", ok=(204,))
        await DRIVER.api.bytes("PUT", f"/containers/{helper}/archive?path=/", content=archive, headers={"Content-Type": "application/x-tar"}, ok=(200,))
    finally:
        await DRIVER.api.call("DELETE", f"/containers/{helper}?force=true", ok=(204, 404))


@app.post("/v1/snapshot")
async def snapshot(body: dict):
    result = {}
    for resource_id in body.get("resource_ids", []):
        archive = await _volume_archive(str(resource_id))
        if len(archive) > 128 * 1024 * 1024:
            raise HTTPException(422, "MCP volume exceeds snapshot limit")
        result[str(resource_id)] = base64.b64encode(archive).decode()
    return {"volumes": result}


@app.post("/v1/validate-snapshot")
async def validate_snapshot(body: dict):
    try:
        for resource_id, archive in body.get("volumes", {}).items():
            _names(resource_id)
            if len(base64.b64decode(archive, validate=True)) > 128 * 1024 * 1024:
                raise ValueError()
    except Exception:
        raise HTTPException(422, "Invalid MCP volume snapshot")
    return {"ok": True}


@app.post("/v1/restore-snapshot")
async def restore_snapshot(body: dict):
    await validate_snapshot(body)
    for resource_id, archive in body.get("volumes", {}).items():
        await _restore_archive(resource_id, base64.b64decode(archive))
    return {"ok": True}


@app.post("/v1/install")
async def install(body: dict):
    resource_id = str(body.get("resource_id", ""))
    lock = LOCKS.setdefault(resource_id, asyncio.Lock())
    if lock.locked():
        raise HTTPException(409, "MCP lifecycle operation is already running")
    async with lock:
        try:
            return await DRIVER.install(resource_id, body["manifest"], body.get("network_capability", {}))
        except (ValueError, RuntimeError) as error:
            raise HTTPException(422, str(error))


@app.post("/v1/runtime/{resource_id}/{action}")
async def lifecycle(resource_id: str, action: str, body: dict):
    lock = LOCKS.setdefault(resource_id, asyncio.Lock())
    async with lock:
        try:
            return await DRIVER.action(resource_id, action, bool(body.get("delete_volume")))
        except (ValueError, RuntimeError) as error:
            raise HTTPException(422, str(error))


@app.post("/v1/update/{resource_id}")
async def update(resource_id: str, body: dict):
    lock = LOCKS.setdefault(resource_id, asyncio.Lock())
    async with lock:
        try:
            if not isinstance(DRIVER, DockerRuntimeDriver):
                raise ValueError("Runtime does not support transactional updates")
            return await DRIVER.update(resource_id, body["old_manifest"], body["new_manifest"], body.get("network_capability", {}), body.get("health", {}))
        except (ValueError, RuntimeError) as error:
            raise HTTPException(422, str(error))


async def _proxy(resource_id: str, path: str, body: dict) -> dict:
    policy = _load_policy()
    driver = policy.get("resource_policies", {}).get(resource_id, {}).get("driver")
    if driver == "oci":
        if body.get("credentials"):
            raise ValueError("OCI MCP secrets require a bootstrap-compatible image")
        method = "server/discover" if path == "discover" else "tools/call"
        params = {} if path == "discover" else {"name": body["tool"], "arguments": body.get("arguments_value", {})}
        value = await _oci_rpc(resource_id, method, params)
        if path == "discover":
            return {"tools": value.get("tools", value.get("capabilities", {}).get("tools", []))}
        return value
    container, _, _ = _names(resource_id)
    async with httpx.AsyncClient(timeout=120, trust_env=False) as client:
        response = None
        for attempt in range(60):
            try:
                response = await client.post(f"http://{container}:8004/{path}", json=body)
                break
            except httpx.ConnectError:
                if attempt == 59:
                    raise
                await asyncio.sleep(0.5)
        assert response is not None
        response.raise_for_status()
        value = response.json()
    if path == "discover" and isinstance(DRIVER, DockerRuntimeDriver):
        policy = _load_policy()
        resource = policy.get("resource_policies", {}).get(resource_id, {})
        if resource.get("installing"):
            runtime = resource.get("runtime", {"mode": "none", "allowed_domains": []})
            ip = resource.get("ip", "")
            if ip:
                policy.setdefault("source_policies", {})[ip] = runtime
            resource["installing"] = False
            _save_policy(policy)
            if runtime.get("mode") == "none":
                _, network, _ = _names(resource_id)
                try:
                    await DRIVER.api.call("POST", f"/networks/{network}/disconnect", body={"Container": EGRESS_CONTAINER, "Force": False})
                except RuntimeError:
                    pass
    return value


async def _oci_rpc(resource_id: str, method: str, params: dict) -> dict:
    import websockets

    container, _, _ = _names(resource_id)
    channel = OCI_CHANNELS.get(resource_id)
    if not channel:
        uri = f"ws://localhost/v1.47/containers/{container}/attach/ws?stdin=1&stdout=1&stderr=1&stream=1"
        websocket = await websockets.unix_connect(os.getenv("DOCKER_SOCKET", "/var/run/docker.sock"), uri=uri, max_size=16 * 1024 * 1024)
        channel = (websocket, asyncio.Lock(), "modern")
        OCI_CHANNELS[resource_id] = channel
    websocket, lock, generation = channel
    async with lock:
        request_id = next(RPC_IDS)
        request_params = dict(params)
        request_params["_meta"] = {
            "io.modelcontextprotocol/clientInfo": {"name": "MIX agent", "version": "0.1.0"},
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        }
        await websocket.send((json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": request_params}) + "\n").encode())
        for _ in range(100):
            raw = await asyncio.wait_for(websocket.recv(), 115)
            if isinstance(raw, bytes):
                raw = raw.decode(errors="replace")
            for line in raw.splitlines():
                try:
                    value = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if value.get("id") != request_id:
                    continue
                if value.get("error"):
                    raise ValueError("OCI MCP operation failed")
                return value.get("result", {})
        raise ValueError("OCI MCP response was not received")


@app.post("/v1/discover/{resource_id}")
async def discover(resource_id: str, body: dict):
    return await _proxy(resource_id, "discover", body)


@app.post("/v1/call/{resource_id}")
async def call(resource_id: str, body: dict):
    return await _proxy(resource_id, "call", body)
