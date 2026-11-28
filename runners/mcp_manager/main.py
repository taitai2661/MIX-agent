"""Small, policy-constrained Docker lifecycle manager for local MCP workloads."""

from __future__ import annotations

import asyncio
import base64
import hashlib
import io
import ipaddress
import itertools
import json
import logging
import os
import posixpath
import re
import secrets
import tarfile
import tempfile
import uuid
from abc import ABC, abstractmethod
from contextlib import AsyncExitStack, asynccontextmanager
from pathlib import Path

import httpx
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse

app = FastAPI(docs_url=None, redoc_url=None)
LOGGER = logging.getLogger(__name__)
LABEL = "io.mix-agent.mcp"
NAME_RE = re.compile(r"^[a-f0-9-]{1,36}$")
# Hostname-only, like the server's validate_domain(): no IP literals, no
# single-label/metadata names. A dot is required so `169.254.169.254`,
# `localhost` and `metadata` cannot enter an egress allow-list.
DOMAIN_RE = re.compile(
    r"^(?=.{1,253}$)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$"
)
MAX_RESOURCE_LOCKS = 256
MAX_SNAPSHOT_RESOURCES = 20
MAX_SNAPSHOT_TOTAL_BYTES = 256 * 1024 * 1024
OCI_RPC_DEADLINE_SECONDS = 120
RUNTIME_IMAGE = os.getenv("MCP_RUNTIME_IMAGE", "mix-agent-mcp-runtime:local")
MANAGER_CONTAINER = os.getenv("HOSTNAME", "mcp-manager")
EGRESS_CONTAINER = os.getenv("MCP_EGRESS_CONTAINER", "mix-agent-egress-proxy-1")
POLICY = Path(os.getenv("POLICY_FILE", "/policy/mcp-policy.json"))
# The workload receives a dedicated, low-privilege token: it must never be able
# to authenticate to the manager's own admin API by reading its mounted file.
# Safe default: the dedicated workload token, never the manager's admin token.
WORKLOAD_TOKEN_FILE = Path(os.getenv("MCP_WORKLOAD_TOKEN_FILE", "/workload-token/mcp-workload.token"))
WORKLOAD_TOKEN_VOLUME = os.getenv("MCP_WORKLOAD_TOKEN_VOLUME", "mix-agent_mcp-workload-token")
LOCKS: dict[str, asyncio.Lock] = {}
OCI_CHANNEL_LOCK = asyncio.Lock()
# One lock for every policy read-modify-write: per-resource LOCKS only serialise
# lifecycle actions for a single MCP, but install/uninstall/discover all mutate
# the same policy document and used a fixed temp file, losing each other's rules.
POLICY_WRITE_LOCK = asyncio.Lock()
OCI_CHANNELS: dict[str, tuple[object, asyncio.Lock, str]] = {}
RPC_IDS = itertools.count(1)


async def drop_oci_channel(resource_id: str) -> None:
    """Forget the attach stream: a restarted container owns a new process."""
    async with OCI_CHANNEL_LOCK:
        channel = OCI_CHANNELS.pop(resource_id, None)
    if channel:
        try:
            # Take the channel's own lock so we never close a WebSocket in the
            # middle of an in-flight send/recv. A stuck RPC must not block a
            # lifecycle action forever, hence the short deadline.
            async with asyncio.timeout(5), channel[1]:
                await channel[0].close()
        except Exception:  # noqa: BLE001 - the container is gone; close is best-effort
            LOGGER.debug("failed to close OCI channel for %s", resource_id, exc_info=True)


@app.middleware("http")
async def authenticate(request: Request, call_next):
    token = Path(os.getenv("TOKEN_FILE", "/tokens/manager.token")).read_text().strip()
    provided = request.headers.get("authorization", "")
    # compare_digest() raises TypeError for non-ASCII str, which would surface
    # as a 500 on the auth boundary instead of a clean rejection.
    if not provided.isascii() or not secrets.compare_digest(provided, "Bearer " + token):
        return JSONResponse({"detail": "Unauthorized"}, 401)
    return await call_next(request)


class DockerOperationError(RuntimeError):
    """A Docker failure with internal detail for logs/predicates only.

    ``str(error)`` stays generic so endpoint handlers never echo daemon
    internals (container names, paths, image refs) to API clients. Internal
    code that needs to distinguish "already exists" reads ``detail``.
    """

    def __init__(self, detail: str):
        super().__init__("Docker operation failed")
        self.detail = detail


def _docker_detail(error: RuntimeError) -> str:
    return getattr(error, "detail", str(error)).lower()


class ContainerRuntimeDriver(ABC):
    @abstractmethod
    async def install(self, resource_id: str, manifest: dict, capability: dict) -> dict: ...

    @abstractmethod
    async def action(self, resource_id: str, action: str, delete_volume: bool = False) -> dict: ...


def _docker_target() -> tuple[str, str | None]:
    """Return the Docker API base URL and an optional unix-socket path.

    Both the HTTP client and the OCI attach WebSocket must use the *same*
    configured endpoint.  Production sets ``DOCKER_HOST`` to the socket proxy;
    the raw host socket is never mounted into this container. The unix-socket
    branch exists only for local development and host-side tests.
    """
    docker_host = os.getenv("DOCKER_HOST", "").strip()
    if docker_host.startswith("tcp://"):
        docker_host = "http://" + docker_host[len("tcp://") :]
    if docker_host.startswith(("http://", "https://")):
        base_url = docker_host.rstrip("/")
        if not base_url.endswith("/v1.47"):
            base_url += "/v1.47"
        return base_url, None
    socket = docker_host.removeprefix("unix://") if docker_host else os.getenv("DOCKER_SOCKET", "")
    return "http://docker/v1.47", (socket or "/var/run/docker.sock")


def _mount_signature(mount: dict) -> tuple:
    """Normalise create-config and inspect mounts into a comparable tuple.

    Docker's create API uses ``Source``/``Target``/``ReadOnly`` while inspect
    uses ``Source``/``Destination``/``RW``; tmpfs options may also differ in
    representation, so only the size is compared. ``Mode`` is intentionally
    omitted to avoid restarting identical containers on a formatting change.
    """
    options = mount.get("TmpfsOptions") or {}
    readonly = not bool(mount["RW"]) if "RW" in mount else bool(mount.get("ReadOnly"))
    return (
        mount.get("Type"),
        mount.get("Source") or mount.get("Name") or "",
        mount.get("Destination") or mount.get("Target"),
        readonly,
        options.get("SizeBytes"),
    )


def _container_config_matches(existing: dict, config: dict) -> bool:
    """Compare the *inspected* image/env/mounts/security options to the intent.

    Self-reported labels are not sufficient: any peer with access to the Docker
    socket proxy can create a container wearing the expected fingerprint while
    running a different image or mounting a different token source.
    """
    actual = existing.get("Config", {}) or {}
    host = existing.get("HostConfig", {}) or {}
    if actual.get("Image") != config.get("Image"):
        return False
    if list(actual.get("Env") or []) != list(config.get("Env") or []):
        return False
    if actual.get("User") != config.get("User"):
        return False
    if actual.get("WorkingDir") != config.get("WorkingDir"):
        return False
    actual_mounts = sorted((_mount_signature(m) for m in host.get("Mounts") or []), key=repr)
    expected_mounts = sorted(
        (_mount_signature(m) for m in (config.get("HostConfig") or {}).get("Mounts") or []), key=repr
    )
    if actual_mounts != expected_mounts:
        return False
    if sorted(host.get("SecurityOpt") or []) != sorted(
        (config.get("HostConfig") or {}).get("SecurityOpt") or []
    ):
        return False
    return True


class DockerAPI:
    def __init__(self):
        """Talk to Docker through the configured proxy or unix socket."""
        base_url, socket = _docker_target()
        if socket:
            self.client = httpx.AsyncClient(
                transport=httpx.AsyncHTTPTransport(uds=socket),
                base_url=base_url,
                timeout=120,
                trust_env=False,
            )
        else:
            self.client = httpx.AsyncClient(base_url=base_url, timeout=120, trust_env=False)

    async def call(self, method: str, path: str, *, body=None, ok=(200, 201, 204, 304)) -> dict:
        response = await self.client.request(method, path, json=body)
        if response.status_code not in ok:
            message = "Docker operation failed"
            try:
                message = response.json().get("message", message)
            except Exception:  # noqa: BLE001 - keep the generic Docker error message
                LOGGER.debug("Docker error response was not JSON", exc_info=True)
            # Log the detail, but never hand it to an API client: Docker error
            # bodies contain internal container names, paths and image refs.
            LOGGER.warning("Docker API %s %s failed: %s", method, path, message)
            raise DockerOperationError(message)
        if not response.content:
            return {}
        return response.json()

    async def bytes(
        self, method: str, path: str, *, content: bytes | None = None, headers: dict | None = None, ok=(200,)
    ) -> bytes:
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
        # A dotted string can still be an IP literal; the server's
        # validate_domain() rejects those too. Keep both editions aligned.
        try:
            ipaddress.ip_address(host)
        except ValueError:
            pass
        else:
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


async def _save_policy(value: dict) -> None:
    """Low-level atomic+durable write. Callers hold ``POLICY_WRITE_LOCK``."""
    POLICY.parent.mkdir(parents=True, exist_ok=True)
    handle = tempfile.NamedTemporaryFile(
        "w",
        dir=POLICY.parent,
        prefix="mcp-policy.",
        suffix=".tmp",
        delete=False,
        encoding="utf-8",
    )
    temporary = Path(handle.name)
    try:
        # The proxy reads this file as uid 1000; mode 0644 matches egress_policy.
        os.chmod(temporary, 0o644)
        with handle:
            json.dump(value, handle, sort_keys=True)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, POLICY)
    finally:
        temporary.unlink(missing_ok=True)


@asynccontextmanager
async def _policy_update():
    """Serialise a full read-modify-write of the shared policy document."""
    async with POLICY_WRITE_LOCK:
        policy = _load_policy()
        yield policy
        await _save_policy(policy)


def _checked_lock(resource_id: str) -> asyncio.Lock:
    """Validate before touching process state and bound the lock map."""
    _names(resource_id)  # raises ValueError for malformed ids
    if len(LOCKS) >= MAX_RESOURCE_LOCKS and resource_id not in LOCKS:
        raise ValueError("Too many MCP resources are being managed")
    return LOCKS.setdefault(resource_id, asyncio.Lock())


@asynccontextmanager
async def _exclusive(resource_id: str):
    """Non-blocking exclusive lifecycle operation for one resource.

    There is no await between `locked()` and `acquire()`, so a concurrent
    caller cannot sneak in: it gets a clean 409 instead of silently queueing.
    """
    lock = _checked_lock(resource_id)
    if lock.locked():
        raise HTTPException(409, "MCP lifecycle operation is already running")
    await lock.acquire()
    try:
        yield
    finally:
        lock.release()


@asynccontextmanager
async def _resource_locks(resource_ids: list[str]):
    """Acquire all resource locks in a stable order (snapshots/restores)."""
    unique = sorted(set(resource_ids))
    if len(unique) > MAX_SNAPSHOT_RESOURCES:
        raise ValueError("Too many MCP resources in one snapshot operation")
    locks = [_checked_lock(resource_id) for resource_id in unique]
    async with AsyncExitStack() as stack:
        for lock in locks:
            await stack.enter_async_context(lock)
        yield


class DockerRuntimeDriver(ContainerRuntimeDriver):
    def __init__(self):
        self.api = DockerAPI()

    async def _connect(self, network: str, container: str, aliases: list[str] | None = None):
        try:
            await self.api.call(
                "POST",
                f"/networks/{network}/connect",
                body={"Container": container, "EndpointConfig": {"Aliases": aliases or []}},
            )
        except RuntimeError as error:
            if "already exists" not in _docker_detail(error):
                raise

    async def prepare_volume(self, volume: str) -> None:
        helper = "mix-mcp-volume-init-" + uuid.uuid4().hex
        body = {
            "Image": RUNTIME_IMAGE,
            "Cmd": [
                "python",
                "-c",
                "import os,pathlib;[(pathlib.Path('/data')/p).mkdir(exist_ok=True) for p in ('.packages','workspace')];[os.chown('/data/'+p,1000,1000) for p in ('.packages','workspace')]",
            ],
            "User": "0:0",
            "Labels": {LABEL: "volume-init"},
            "HostConfig": {
                "NetworkMode": "none",
                "ReadonlyRootfs": True,
                "CapDrop": ["ALL"],
                "CapAdd": ["CHOWN"],
                "SecurityOpt": ["no-new-privileges:true"],
                "Mounts": [{"Type": "volume", "Source": volume, "Target": "/data", "ReadOnly": False}],
            },
        }
        await self.api.call("POST", f"/containers/create?name={helper}", body=body)
        try:
            await self.api.call("POST", f"/containers/{helper}/start", ok=(204,))
            result = await self.api.call("POST", f"/containers/{helper}/wait?condition=not-running")
            if result.get("StatusCode"):
                raise RuntimeError("MCP volume initialization failed")
        finally:
            await self.api.call("DELETE", f"/containers/{helper}?force=true", ok=(204, 404))

    async def _ensure_container(self, name: str, config: dict, resource_id: str, kind: str) -> None:
        """Create ``name``, or only reuse a container we can prove is identical.

        Swallowing Docker's "already in use" error and starting whatever happens
        to own the name could run a different image or config (and bypass the
        workload token mount). Foreign containers are rejected; a stale install
        of this same resource is deleted and recreated. The fingerprint label is
        advisory only: a peer with Docker API access can forge labels, so the
        *actual* inspect data must match the config we intended to create.
        """
        try:
            await self.api.call("POST", f"/containers/create?name={name}", body=config)
            return
        except RuntimeError as error:
            if "already in use" not in _docker_detail(error):
                raise
        existing = await self.api.call("GET", f"/containers/{name}/json")
        existing_config = existing.get("Config", {})
        labels = existing_config.get("Labels") or {}
        if labels.get(LABEL) != resource_id or labels.get(f"{LABEL}.driver") != kind:
            raise RuntimeError("MCP container name is owned by a different workload")
        if _container_config_matches(existing, config):
            return
        # Our resource, but a different install: recreate instead of starting a
        # container whose image, env or token mount we did not verify.
        await self.api.call("DELETE", f"/containers/{name}?force=true", ok=(204, 404))
        await self.api.call("POST", f"/containers/create?name={name}", body=config)

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
            await self.api.call(
                "POST",
                "/networks/create",
                body={"Name": network, "Internal": True, "Labels": {LABEL: resource_id}},
            )
        except RuntimeError as error:
            if "already exists" not in _docker_detail(error):
                raise
        image = candidate["identifier"] if kind == "oci" else RUNTIME_IMAGE
        env = [
            f"MCP_PACKAGE_KIND={kind}",
            f"MCP_PACKAGE_IDENTIFIER={candidate['identifier']}",
            f"MCP_PACKAGE_VERSION={candidate.get('version', manifest.get('version', ''))}",
            "HOME=/data/workspace",
            "TMPDIR=/tmp",
            "PYTHONPATH=/app",
            f"TOKEN_FILE=/tokens/{WORKLOAD_TOKEN_FILE.name}",
        ]
        if capability["mode"] != "none" or kind in ("npm", "pypi"):
            env.extend(
                [
                    "HTTP_PROXY=http://egress-proxy:3128",
                    "HTTPS_PROXY=http://egress-proxy:3128",
                    "http_proxy=http://egress-proxy:3128",
                    "https_proxy=http://egress-proxy:3128",
                ]
            )
        config = {
            "Image": image,
            "Env": env,
            "Labels": {
                LABEL: resource_id,
                f"{LABEL}.driver": kind,
                # Advisory fingerprint for observability. Reuse is decided by
                # _container_config_matches() against the real inspect data;
                # labels alone can be forged by another Docker API caller.
                f"{LABEL}.fingerprint": hashlib.sha256(
                    json.dumps(
                        {"image": image, "env": env, "token_volume": WORKLOAD_TOKEN_VOLUME}, sort_keys=True
                    ).encode()
                ).hexdigest()[:24],
            },
            "User": "1000:1000",
            "WorkingDir": "/data",
            "HostConfig": {
                "ReadonlyRootfs": True,
                "CapDrop": ["ALL"],
                "SecurityOpt": ["no-new-privileges:true"],
                "NetworkMode": network,
                "Mounts": [
                    {"Type": "volume", "Source": volume, "Target": "/data", "ReadOnly": False},
                    {
                        "Type": "volume",
                        "Source": WORKLOAD_TOKEN_VOLUME,
                        "Target": "/tokens",
                        "ReadOnly": True,
                    },
                    {
                        "Type": "tmpfs",
                        "Target": "/tmp",
                        "TmpfsOptions": {"SizeBytes": 268435456, "Mode": 0o1777},
                    },
                    {
                        "Type": "tmpfs",
                        "Target": "/run/secrets/mcp",
                        "TmpfsOptions": {"SizeBytes": 1048576, "Mode": 0o700},
                    },
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
        await self._ensure_container(container, config, resource_id, kind)
        await self._connect(network, MANAGER_CONTAINER, ["mcp-manager"])
        needs_install_egress = kind in ("npm", "pypi")
        if capability["mode"] != "none" or needs_install_egress:
            await self._connect(network, EGRESS_CONTAINER, ["egress-proxy"])
        inspect = await self.api.call("GET", f"/containers/{container}/json")
        ip = inspect.get("NetworkSettings", {}).get("Networks", {}).get(network, {}).get("IPAddress", "")
        async with _policy_update() as policy:
            policies = policy.setdefault("source_policies", {})
            if ip:
                policies[ip] = (
                    (
                        {"mode": "restricted", "allowed_domains": ["registry.npmjs.org"]}
                        if kind == "npm"
                        else {"mode": "restricted", "allowed_domains": ["pypi.org", "files.pythonhosted.org"]}
                    )
                    if needs_install_egress
                    else capability
                )
            policy.setdefault("resource_policies", {})[resource_id] = {
                "ip": ip,
                "runtime": capability,
                "installing": needs_install_egress,
                "driver": kind,
            }
        await self.api.call("POST", f"/containers/{container}/start", ok=(204, 304))
        return {
            "container": container,
            "network": network,
            "volume": volume,
            "driver": kind,
            "state": "running",
        }

    async def action(self, resource_id: str, action: str, delete_volume: bool = False) -> dict:
        container, network, volume = _names(resource_id)
        if action in ("start", "stop", "restart"):
            await self.api.call("POST", f"/containers/{container}/{action}?t=20", ok=(204, 304))
            # The websocket was attached to the previous process. Reusing it
            # after a lifecycle change made discover/call fail forever.
            await drop_oci_channel(resource_id)
        elif action == "status":
            value = await self.api.call("GET", f"/containers/{container}/json")
            return {
                "state": value.get("State", {}),
                "container": container,
                "network": network,
                "volume": volume,
            }
        elif action == "uninstall":
            try:
                await self.api.call("POST", f"/containers/{container}/stop?t=20", ok=(204, 304))
            except RuntimeError:
                pass
            await self.api.call("DELETE", f"/containers/{container}?force=false&v=false", ok=(204, 404))
            for attached in (MANAGER_CONTAINER, EGRESS_CONTAINER):
                try:
                    await self.api.call(
                        "POST", f"/networks/{network}/disconnect", body={"Container": attached, "Force": True}
                    )
                except RuntimeError:
                    pass
            await self.api.call("DELETE", f"/networks/{network}", ok=(204, 404))
            async with _policy_update() as policy:
                resource = policy.get("resource_policies", {}).pop(resource_id, {})
                if resource.get("ip"):
                    policy.get("source_policies", {}).pop(resource["ip"], None)
            await drop_oci_channel(resource_id)
            if delete_volume:
                await self.api.call("DELETE", f"/volumes/{volume}", ok=(204, 404))
        else:
            raise ValueError("Unknown runtime action")
        return {"ok": True, "container": container, "network": network, "volume": volume}

    async def copy_volume(self, source: str, destination: str, *, clear: bool = False) -> None:
        helper = "mix-mcp-copy-" + uuid.uuid4().hex
        script = (
            "import pathlib,shutil; src=pathlib.Path('/source'); dst=pathlib.Path('/destination'); "
            + (
                "[shutil.rmtree(p) if p.is_dir() else p.unlink() for p in list(dst.iterdir())]; "
                if clear
                else ""
            )
            + "[shutil.copytree(p,dst/p.name,symlinks=False) if p.is_dir() else shutil.copy2(p,dst/p.name) for p in src.iterdir()]"
        )
        body = {
            "Image": RUNTIME_IMAGE,
            "Cmd": ["python", "-c", script],
            "User": "0:0",
            "Labels": {LABEL: "volume-copy"},
            "HostConfig": {
                "NetworkMode": "none",
                "ReadonlyRootfs": True,
                "CapDrop": ["ALL"],
                "SecurityOpt": ["no-new-privileges:true"],
                "Mounts": [
                    {"Type": "volume", "Source": source, "Target": "/source", "ReadOnly": True},
                    {"Type": "volume", "Source": destination, "Target": "/destination", "ReadOnly": False},
                ],
            },
        }
        await self.api.call("POST", f"/containers/create?name={helper}", body=body)
        try:
            await self.api.call("POST", f"/containers/{helper}/start", ok=(204,))
            result = await self.api.call("POST", f"/containers/{helper}/wait?condition=not-running")
            if result.get("StatusCode"):
                raise RuntimeError("Volume snapshot failed")
        finally:
            await self.api.call("DELETE", f"/containers/{helper}?force=true", ok=(204, 404))

    async def update(
        self, resource_id: str, old_manifest: dict, new_manifest: dict, capability: dict, health: dict
    ) -> dict:
        _, _, volume = _names(resource_id)
        backup = volume + "-backup-" + uuid.uuid4().hex[:12]
        await self.api.call(
            "POST",
            "/volumes/create",
            body={"Name": backup, "Labels": {LABEL: resource_id, f"{LABEL}.backup": "true"}},
        )
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
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            try:
                await self.action(resource_id, "uninstall", False)
            except Exception:  # noqa: BLE001 - rollback continues even if uninstall fails
                LOGGER.debug("uninstall during rollback failed for %s", resource_id, exc_info=True)
            await self.copy_volume(backup, volume, clear=True)
            await self.install(resource_id, old_manifest, capability)
            raise RuntimeError("MCP update failed; previous version was restored")


DRIVER: ContainerRuntimeDriver = DockerRuntimeDriver()


async def _volume_archive(resource_id: str) -> bytes:
    if not isinstance(DRIVER, DockerRuntimeDriver):
        raise ValueError("Runtime does not support snapshots")  # noqa: TRY004 - ValueError is this app's domain-error convention (mapped to HTTP 422)
    _, _, volume = _names(resource_id)
    helper = "mix-mcp-snapshot-" + uuid.uuid4().hex
    body = {
        "Image": RUNTIME_IMAGE,
        "Cmd": ["python", "-c", "import time;time.sleep(300)"],
        "User": "0:0",
        "Labels": {LABEL: "snapshot"},
        "HostConfig": {
            "NetworkMode": "none",
            "ReadonlyRootfs": True,
            "CapDrop": ["ALL"],
            "SecurityOpt": ["no-new-privileges:true"],
            "Mounts": [{"Type": "volume", "Source": volume, "Target": "/data", "ReadOnly": False}],
        },
    }
    await DRIVER.api.call("POST", f"/containers/create?name={helper}", body=body)
    try:
        await DRIVER.api.call("POST", f"/containers/{helper}/start", ok=(204,))
        return await DRIVER.api.bytes("GET", f"/containers/{helper}/archive?path=/data")
    finally:
        await DRIVER.api.call("DELETE", f"/containers/{helper}?force=true", ok=(204, 404))


def _safe_archive(archive: bytes) -> None:
    """Reject tar members that could write outside the restored volume."""
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
        for member in tar.getmembers():
            name = str(member.name).replace("\\", "/")
            if name.startswith("/") or any(part == ".." for part in name.split("/")):
                raise ValueError("Unsafe path in MCP volume snapshot")
            if not (member.isfile() or member.isdir() or member.issym() or member.islnk()):
                raise ValueError("Unsupported member in MCP volume snapshot")
            if member.issym() or member.islnk():
                target = str(member.linkname).replace("\\", "/")
                if target.startswith("/"):
                    raise ValueError("Unsafe link in MCP volume snapshot")
                resolved = posixpath.normpath(posixpath.join(posixpath.dirname(name), target))
                if resolved == ".." or resolved.startswith("../") or resolved.startswith("/"):
                    raise ValueError("Unsafe link in MCP volume snapshot")


def _decoded_snapshot_volumes(body: dict) -> dict[str, bytes]:
    volumes = body.get("volumes", {})
    if not isinstance(volumes, dict) or len(volumes) > MAX_SNAPSHOT_RESOURCES:
        raise ValueError("Invalid MCP volume snapshot")
    decoded: dict[str, bytes] = {}
    total = 0
    for resource_id, archive in volumes.items():
        _names(str(resource_id))
        raw = base64.b64decode(archive, validate=True)
        _safe_archive(raw)
        total += len(raw)
        if len(raw) > 128 * 1024 * 1024 or total > MAX_SNAPSHOT_TOTAL_BYTES:
            raise ValueError("MCP volume exceeds snapshot limit")
        decoded[str(resource_id)] = raw
    return decoded


async def _restore_archive(resource_id: str, archive: bytes) -> None:
    if not isinstance(DRIVER, DockerRuntimeDriver):
        raise ValueError("Runtime does not support snapshots")  # noqa: TRY004 - ValueError is this app's domain-error convention (mapped to HTTP 422)
    _, _, volume = _names(resource_id)
    await DRIVER.api.call("POST", "/volumes/create", body={"Name": volume, "Labels": {LABEL: resource_id}})
    helper = "mix-mcp-restore-" + uuid.uuid4().hex
    body = {
        "Image": RUNTIME_IMAGE,
        "Cmd": ["python", "-c", "import time;time.sleep(300)"],
        "User": "0:0",
        "Labels": {LABEL: "restore"},
        "HostConfig": {
            "NetworkMode": "none",
            "ReadonlyRootfs": True,
            "CapDrop": ["ALL"],
            "SecurityOpt": ["no-new-privileges:true"],
            "Mounts": [{"Type": "volume", "Source": volume, "Target": "/data", "ReadOnly": False}],
        },
    }
    await DRIVER.api.call("POST", f"/containers/create?name={helper}", body=body)
    try:
        await DRIVER.api.call("POST", f"/containers/{helper}/start", ok=(204,))
        await DRIVER.api.bytes(
            "PUT",
            f"/containers/{helper}/archive?path=/",
            content=archive,
            headers={"Content-Type": "application/x-tar"},
            ok=(200,),
        )
    finally:
        await DRIVER.api.call("DELETE", f"/containers/{helper}?force=true", ok=(204, 404))


@app.post("/v1/snapshot")
async def snapshot(body: dict):
    ids = [str(value) for value in body.get("resource_ids", [])]
    result = {}
    try:
        # Hold the same locks lifecycle operations use: snapshotting a volume
        # mid-install would produce an inconsistent, unrecoverable artifact.
        async with _resource_locks(ids):
            total = 0
            for resource_id in ids:
                archive = await _volume_archive(resource_id)
                total += len(archive)
                if len(archive) > 128 * 1024 * 1024 or total > MAX_SNAPSHOT_TOTAL_BYTES:
                    raise ValueError("MCP volume exceeds snapshot limit")
                result[resource_id] = base64.b64encode(archive).decode()
    except (ValueError, TypeError, RuntimeError) as error:
        raise HTTPException(422, str(error))
    return {"volumes": result}


@app.post("/v1/validate-snapshot")
async def validate_snapshot(body: dict):
    try:
        _decoded_snapshot_volumes(body)
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        raise HTTPException(422, "Invalid MCP volume snapshot")
    return {"ok": True}


@app.post("/v1/restore-snapshot")
async def restore_snapshot(body: dict):
    try:
        volumes = _decoded_snapshot_volumes(body)
        resource_ids = sorted(volumes)
        async with _resource_locks(resource_ids):
            for resource_id in resource_ids:
                await _restore_archive(resource_id, volumes[resource_id])
    except (ValueError, TypeError, RuntimeError) as error:
        raise HTTPException(422, str(error))
    return {"ok": True}


@app.post("/v1/install")
async def install(body: dict):
    resource_id = str(body.get("resource_id", ""))
    try:
        async with _exclusive(resource_id):
            return await DRIVER.install(resource_id, body["manifest"], body.get("network_capability", {}))
    except (ValueError, RuntimeError) as error:
        raise HTTPException(422, str(error))


@app.post("/v1/runtime/{resource_id}/{action}")
async def lifecycle(resource_id: str, action: str, body: dict):
    try:
        async with _exclusive(resource_id):
            return await DRIVER.action(resource_id, action, bool(body.get("delete_volume")))
    except (ValueError, RuntimeError) as error:
        raise HTTPException(422, str(error))


@app.post("/v1/update/{resource_id}")
async def update(resource_id: str, body: dict):
    try:
        async with _exclusive(resource_id):
            if not isinstance(DRIVER, DockerRuntimeDriver):
                raise ValueError("Runtime does not support transactional updates")  # noqa: TRY004 - ValueError is this app's domain-error convention (mapped to HTTP 422)
            return await DRIVER.update(
                resource_id,
                body["old_manifest"],
                body["new_manifest"],
                body.get("network_capability", {}),
                body.get("health", {}),
            )
    except (ValueError, RuntimeError) as error:
        raise HTTPException(422, str(error))


async def _proxy(resource_id: str, path: str, body: dict) -> dict:
    policy = _load_policy()
    driver = policy.get("resource_policies", {}).get(resource_id, {}).get("driver")
    if driver == "oci":
        if body.get("credentials"):
            raise ValueError("OCI MCP secrets require a bootstrap-compatible image")
        method = "server/discover" if path == "discover" else "tools/call"
        params = (
            {} if path == "discover" else {"name": body["tool"], "arguments": body.get("arguments_value", {})}
        )
        value = await _oci_rpc(resource_id, method, params)
        if path == "discover":
            return {"tools": value.get("tools", value.get("capabilities", {}).get("tools", []))}
        return value
    container, _, _ = _names(resource_id)
    token = WORKLOAD_TOKEN_FILE.read_text().strip()
    async with httpx.AsyncClient(timeout=120, trust_env=False) as client:
        response = None
        for attempt in range(60):
            try:
                response = await client.post(
                    f"http://{container}:8004/{path}",
                    json=body,
                    headers={"Authorization": "Bearer " + token},
                )
                break
            except httpx.ConnectError:
                if attempt == 59:
                    raise
                await asyncio.sleep(0.5)
        if response is None:
            raise RuntimeError(f"workload {resource_id} never answered {path}")
        response.raise_for_status()
        value = response.json()
    if path == "discover" and isinstance(DRIVER, DockerRuntimeDriver):
        runtime = None
        async with _policy_update() as policy:
            resource = policy.get("resource_policies", {}).get(resource_id, {})
            if resource.get("installing"):
                runtime = resource.get("runtime", {"mode": "none", "allowed_domains": []})
                ip = resource.get("ip", "")
                if ip:
                    policy.setdefault("source_policies", {})[ip] = runtime
                resource["installing"] = False
        if runtime is not None and runtime.get("mode") == "none":
            _, network, _ = _names(resource_id)
            try:
                await DRIVER.api.call(
                    "POST",
                    f"/networks/{network}/disconnect",
                    body={"Container": EGRESS_CONTAINER, "Force": False},
                )
            except RuntimeError:
                pass
    return value


async def _oci_rpc(resource_id: str, method: str, params: dict) -> dict:
    import websockets

    container, _, _ = _names(resource_id)
    # Two concurrent first calls must not each open an attach stream: one
    # would be orphaned and leak a Docker connection forever.
    async with OCI_CHANNEL_LOCK:
        channel = OCI_CHANNELS.get(resource_id)
        if not channel:
            base_url, socket = _docker_target()
            endpoint = base_url.replace("https://", "wss://", 1).replace("http://", "ws://", 1)
            uri = f"{endpoint}/containers/{container}/attach/ws?stdin=1&stdout=1&stderr=1&stream=1"
            # Attach over the same proxied DOCKER_HOST the HTTP API uses. The raw
            # host socket is never mounted here; a unix path is only a local-dev
            # fallback and is passed via ``unix_connect`` without any http proxy.
            if socket:
                websocket = await websockets.unix_connect(socket, uri=uri, max_size=16 * 1024 * 1024)
            else:
                websocket = await websockets.connect(uri, max_size=16 * 1024 * 1024)
            channel = (websocket, asyncio.Lock(), "modern")
            OCI_CHANNELS[resource_id] = channel
    websocket, lock, _ = channel
    async with lock:
        request_id = next(RPC_IDS)
        request_params = dict(params)
        request_params["_meta"] = {
            "io.modelcontextprotocol/clientInfo": {"name": "MIX agent", "version": "0.5.8"},
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        }
        await websocket.send(
            (
                json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": request_params})
                + "\n"
            ).encode()
        )
        # A bounded deadline replaces the previous 100 x 115s (~3h) loop,
        # which could pin the manager's only channel lock long after callers
        # had timed out.
        deadline = asyncio.get_running_loop().time() + OCI_RPC_DEADLINE_SECONDS
        while True:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise ValueError("OCI MCP response was not received")
            try:
                raw = await asyncio.wait_for(websocket.recv(), min(15, remaining))
            except TimeoutError:
                continue
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
