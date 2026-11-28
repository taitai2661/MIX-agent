"""Regression coverage for the runner network boundaries (findings 1-4, 12).

These tests run without Docker: the Docker API and the attach WebSocket are
replaced by in-process fakes. A Docker-gated smoke test additionally validates
the rendered Compose configuration when the CLI is available.
"""

import asyncio
import importlib
import json
import shutil
import subprocess
from pathlib import Path
from unittest.mock import AsyncMock

import pytest
import websockets
import yaml
from fastapi.testclient import TestClient
from runners.mcp_manager import main as manager
from runners.mcp_workload import main as workload

ROOT = Path(__file__).resolve().parents[2]


@pytest.fixture
def token_file(tmp_path, monkeypatch):
    path = tmp_path / "mcp-workload.token"
    path.write_text("workload-secret\n")
    monkeypatch.setenv("TOKEN_FILE", str(path))
    monkeypatch.setattr(manager, "WORKLOAD_TOKEN_FILE", path)
    return path


class _Result:
    def model_dump(self, **_kwargs):
        return {"content": [{"type": "text", "text": "ok"}]}


class _Session:
    async def call_tool(self, _name, _arguments):
        return _Result()


async def _session_stub(_credentials, _arguments):
    return _Session()


def test_workload_rejects_unauthenticated_call(token_file, monkeypatch):
    monkeypatch.setattr(workload, "_session", _session_stub)
    with TestClient(workload.app) as client:
        response = client.post("/call", json={"tool": "echo", "arguments_value": {}})
    assert response.status_code == 401
    assert response.json() == {"detail": "Unauthorized"}


def test_workload_rejects_wrong_token(token_file, monkeypatch):
    monkeypatch.setattr(workload, "_session", _session_stub)
    with TestClient(workload.app) as client:
        response = client.post(
            "/call",
            json={"tool": "echo", "arguments_value": {}},
            headers={"Authorization": "Bearer wrong"},
        )
    assert response.status_code == 401


def test_workload_accepts_manager_token(token_file, monkeypatch):
    monkeypatch.setattr(workload, "_session", _session_stub)
    with TestClient(workload.app) as client:
        response = client.post(
            "/call",
            json={"tool": "echo", "arguments_value": {}},
            headers={"Authorization": "Bearer workload-secret"},
        )
    assert response.status_code == 200
    assert response.json()["content"][0]["text"] == "ok"


@pytest.mark.parametrize(
    ("module_path", "middleware_name"),
    [
        ("runners.mcp_manager.main", "authenticate"),
        ("runners.mcp_workload.main", "authenticate"),
        ("runners.mcp.main", "auth"),
        ("runners.execution.main", "auth"),
        ("runners.browser_provisioner.main", "authenticate"),
    ],
)
async def test_non_ascii_authorization_header_is_rejected_with_401(module_path, middleware_name, token_file):
    """compare_digest() raises TypeError on non-ASCII strings (a 500)."""
    module = importlib.import_module(module_path)

    class Request:
        headers = {"authorization": "Bearer \u00e9"}

    async def unexpected_call_next(_request):
        pytest.fail("a malformed header must be rejected before routing")

    response = await getattr(module, middleware_name)(Request(), unexpected_call_next)
    assert response.status_code == 401


async def test_manager_sends_workload_token_on_proxy_call(token_file, tmp_path, monkeypatch):
    policy = tmp_path / "mcp-policy.json"
    policy.write_text(json.dumps({"resource_policies": {"a" * 8: {"driver": "npm", "ip": "172.20.0.5"}}}))
    monkeypatch.setattr(manager, "POLICY", policy)

    captured = {}

    class Response:
        def raise_for_status(self):
            pass

        def json(self):
            return {"ok": True}

    class Client:
        def __init__(self, **_kwargs):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *_exc):
            return False

        async def post(self, url, **kwargs):
            captured.update(url=url, **kwargs)
            return Response()

    monkeypatch.setattr(manager.httpx, "AsyncClient", Client)
    assert await manager._proxy("a" * 8, "call", {"tool": "echo"}) == {"ok": True}
    assert captured["headers"]["Authorization"] == "Bearer workload-secret"


async def test_oci_attach_uses_proxied_docker_host_not_unix_socket(monkeypatch):
    paths = []

    async def handler(websocket):
        paths.append(websocket.request.path)
        async for raw in websocket:
            message = json.loads(raw)
            await websocket.send(
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": message["id"],
                        "result": {"tools": [{"name": "oci-tool"}]},
                    }
                )
            )

    async with websockets.serve(handler, "127.0.0.1", 0) as server:
        port = server.sockets[0].getsockname()[1]
        monkeypatch.setenv("DOCKER_HOST", f"http://127.0.0.1:{port}")
        monkeypatch.setattr(
            websockets,
            "unix_connect",
            lambda *_a, **_k: pytest.fail("the raw Docker socket must never be used"),
        )
        resource_id = "b" * 8
        try:
            result = await manager._oci_rpc(resource_id, "server/discover", {})
        finally:
            channel = manager.OCI_CHANNELS.pop(resource_id, None)
            if channel:
                await channel[0].close()
        assert result["tools"] == [{"name": "oci-tool"}]
        assert paths and paths[0].startswith("/v1.47/containers/mix-mcp-bbbbbbbb/attach/ws")


async def test_lifecycle_change_invalidates_oci_channel(monkeypatch):
    """A restarted container has a new process; the old attach stream is dead."""
    resource = "c" * 8
    closed = []

    class Channel:
        async def close(self):
            closed.append(True)

    manager.OCI_CHANNELS[resource] = (Channel(), asyncio.Lock(), "modern")
    monkeypatch.setattr(manager.DRIVER.api, "call", AsyncMock(return_value={}))
    try:
        await manager.DRIVER.action(resource, "restart")
    finally:
        manager.OCI_CHANNELS.pop(resource, None)
    assert closed == [True]
    assert resource not in manager.OCI_CHANNELS


async def test_docker_target_parses_tcp_and_unix_hosts(monkeypatch):
    monkeypatch.setenv("DOCKER_HOST", "tcp://docker-proxy:2375")
    assert manager._docker_target() == ("http://docker-proxy:2375/v1.47", None)
    monkeypatch.setenv("DOCKER_HOST", "unix:///tmp/test.sock")
    assert manager._docker_target() == ("http://docker/v1.47", "/tmp/test.sock")


def test_compose_only_grants_required_docker_verbs():
    compose = yaml.safe_load((ROOT / "compose.yaml").read_text())
    proxy = compose["services"]["docker-socket-proxy"]["environment"]
    assert proxy["POST"] == 1
    assert proxy["ALLOW_START"] == 1
    assert proxy["ALLOW_STOP"] == 1
    assert proxy["ALLOW_RESTARTS"] == 1
    assert proxy["EXEC"] == 0
    assert proxy["BUILD"] == 0
    manager_service = compose["services"]["mcp-manager"]
    assert manager_service["pull_policy"] == "build"
    assert manager_service["environment"]["MCP_WORKLOAD_TOKEN_FILE"]
    assert all("docker.sock" not in str(mount) for mount in manager_service["volumes"])


def test_mcp_runner_cannot_reach_docker_socket_proxy():
    """The broad Docker verbs must not be reachable from unrelated services.

    ALLOW_START/STOP/RESTARTS cannot be scoped to particular containers, so the
    proxy sits on a dedicated network whose only peer is mcp-manager. A
    compromised mcp-runner (or a workload-side egress proxy) must not be able
    to stop the database or restart arbitrary containers.
    """
    compose = yaml.safe_load((ROOT / "compose.yaml").read_text())
    services = compose["services"]
    proxy_networks = set(services["docker-socket-proxy"]["networks"])
    assert proxy_networks == {"docker-proxy"}
    assert "docker-proxy" in compose["networks"]
    assert compose["networks"]["docker-proxy"]["internal"] is True

    assert "docker-proxy" in services["mcp-manager"]["networks"]
    assert "mcp" in services["mcp-manager"]["networks"]
    for unrelated in ("mcp-runner", "egress-proxy", "app"):
        assert "docker-proxy" not in services[unrelated]["networks"], unrelated


@pytest.mark.skipif(shutil.which("docker") is None, reason="docker CLI is not installed")
def test_compose_config_renders():
    rendered = subprocess.run(  # noqa: S603
        [shutil.which("docker"), "compose", "config", "--quiet"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if rendered.returncode != 0:
        pytest.skip(f"docker daemon/CLI unavailable: {rendered.stderr.strip()}")
    assert rendered.returncode == 0


async def test_install_registers_a_pinned_npm_package(tmp_path, monkeypatch):
    """A pinned npm MCP is installed with a verified, token-mounted container."""
    monkeypatch.setattr(manager, "POLICY", tmp_path / "mcp-policy.json")
    monkeypatch.setattr(manager, "WORKLOAD_TOKEN_VOLUME", "test-workload-token")

    calls = []

    class FakeAPI:
        async def call(self, method, path, *, body=None, ok=(200,)):
            calls.append((method, path, body))
            if path.endswith("/json"):
                return {
                    "NetworkSettings": {"Networks": {"mix-mcp-cccccccc-net": {"IPAddress": "172.20.0.9"}}}
                }
            if "wait?" in path:
                return {"StatusCode": 0}
            return {}

    driver = manager.DockerRuntimeDriver()
    driver.api = FakeAPI()
    manifest = {
        "selected": {
            "kind": "npm",
            "identifier": "@modelcontextprotocol/server-filesystem",
            "version": "2025.8.21",
        }
    }
    result = await driver.install("c" * 8, manifest, {"mode": "none", "allowed_domains": []})
    created = next(
        body for method, path, body in calls if method == "POST" and "/create?name=mix-mcp-cccccccc" in path
    )
    assert "MCP_PACKAGE_IDENTIFIER=@modelcontextprotocol/server-filesystem" in created["Env"]
    assert "MCP_PACKAGE_VERSION=2025.8.21" in created["Env"]
    assert f"TOKEN_FILE=/tokens/{manager.WORKLOAD_TOKEN_FILE.name}" in created["Env"]
    assert any(mount.get("Target") == "/tokens" for mount in created["HostConfig"]["Mounts"])
    assert f"{manager.LABEL}.fingerprint" in created["Labels"]
    assert any(method == "POST" and path.endswith("/start") for method, path, _ in calls)
    assert result["driver"] == "npm"


async def test_existing_container_is_only_reused_when_verified():
    resource_id = "d" * 8
    fingerprint = "a" * 24
    config = {
        "Image": "example:1",
        "Env": ["A=B"],
        "User": "1000:1000",
        "WorkingDir": "/data",
        "Labels": {
            manager.LABEL: resource_id,
            f"{manager.LABEL}.driver": "npm",
            f"{manager.LABEL}.fingerprint": fingerprint,
        },
        "HostConfig": {
            "SecurityOpt": ["no-new-privileges:true"],
            "Mounts": [
                {"Type": "volume", "Source": "data-vol", "Target": "/data", "ReadOnly": False},
                {"Type": "volume", "Source": "workload-token", "Target": "/tokens", "ReadOnly": True},
            ],
        },
    }

    class FakeAPI:
        def __init__(self, labels, fp, *, image="example:1", env=None, mounts=None, security=None):
            self.calls = []
            self.labels = labels
            self.fp = fp
            self.image = image
            self.env = ["A=B"] if env is None else env
            self.mounts = config["HostConfig"]["Mounts"] if mounts is None else mounts
            self.security = ["no-new-privileges:true"] if security is None else security
            self.creates = 0

        async def call(self, method, path, *, body=None, ok=()):
            self.calls.append((method, path))
            if method == "POST" and "/create?" in path:
                self.creates += 1
                if self.creates == 1:
                    raise RuntimeError("Conflict. The name is already in use")
            if method == "GET":
                return {
                    "Config": {
                        "Image": self.image,
                        "Env": self.env,
                        "User": "1000:1000",
                        "WorkingDir": "/data",
                        "Labels": {**self.labels, f"{manager.LABEL}.fingerprint": self.fp},
                    },
                    "HostConfig": {"Mounts": self.mounts, "SecurityOpt": self.security},
                }
            return {}

    driver = manager.DockerRuntimeDriver()
    own_labels = {manager.LABEL: resource_id, f"{manager.LABEL}.driver": "npm"}
    driver.api = FakeAPI(own_labels, fingerprint)
    await driver._ensure_container("mix-mcp-dddddddd", config, resource_id, "npm")
    assert not any(method == "DELETE" for method, _ in driver.api.calls)

    # A forged fingerprint label must not let a different image be reused.
    driver.api = FakeAPI(own_labels, fingerprint, image="evil:latest")
    await driver._ensure_container("mix-mcp-dddddddd", config, resource_id, "npm")
    assert any(method == "DELETE" for method, _ in driver.api.calls)

    driver.api = FakeAPI(own_labels, fingerprint, env=["A=B", "EVIL=1"])
    await driver._ensure_container("mix-mcp-dddddddd", config, resource_id, "npm")
    assert any(method == "DELETE" for method, _ in driver.api.calls)

    # A different token mount (the security-critical case) must be recreated.
    bad_mounts = [
        {"Type": "volume", "Source": "data-vol", "Target": "/data", "ReadOnly": False},
        {"Type": "volume", "Source": "attacker-token", "Target": "/tokens", "ReadOnly": True},
    ]
    driver.api = FakeAPI(own_labels, fingerprint, mounts=bad_mounts)
    await driver._ensure_container("mix-mcp-dddddddd", config, resource_id, "npm")
    assert any(method == "DELETE" for method, _ in driver.api.calls)

    driver.api = FakeAPI(own_labels, fingerprint, security=[])
    await driver._ensure_container("mix-mcp-dddddddd", config, resource_id, "npm")
    assert any(method == "DELETE" for method, _ in driver.api.calls)

    driver.api = FakeAPI({manager.LABEL: "someone-else", f"{manager.LABEL}.driver": "oci"}, fingerprint)
    with pytest.raises(RuntimeError, match="different workload"):
        await driver._ensure_container("mix-mcp-dddddddd", config, resource_id, "npm")


@pytest.mark.asyncio
async def test_policy_writes_are_serialised(tmp_path, monkeypatch):
    monkeypatch.setattr(manager, "POLICY", tmp_path / "mcp-policy.json")

    async def update(index):
        async with manager._policy_update() as policy:
            policy.setdefault("resource_policies", {})[f"r{index}"] = {"index": index}
            await asyncio.sleep(0)

    await asyncio.gather(*(update(index) for index in range(25)))
    value = json.loads(manager.POLICY.read_text())
    assert set(value["resource_policies"]) == {f"r{index}" for index in range(25)}
    assert list(tmp_path.glob("mcp-policy.*.tmp")) == []


@pytest.mark.parametrize("domain", ["169.254.169.254", "localhost", "metadata", "10.0.0.1"])
def test_mcp_domain_allowlist_rejects_non_hostnames(domain):
    with pytest.raises(ValueError, match="allowed domain"):
        manager._capability({"mode": "restricted", "allowed_domains": [domain]})


def test_docker_error_text_is_not_exposed_to_callers():
    error = manager.DockerOperationError("Conflict: /var/lib/docker/volumes/secret/name")
    assert str(error) == "Docker operation failed"
    assert manager._docker_detail(error) == "conflict: /var/lib/docker/volumes/secret/name"


def test_snapshot_archive_rejects_escaping_paths_and_links():
    import io
    import tarfile

    def archive(name, linkname=None):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w") as tar:
            info = tarfile.TarInfo(name)
            info.type = tarfile.SYMTYPE if linkname else tarfile.REGTYPE
            info.linkname = linkname or ""
            tar.addfile(info)
        return buffer.getvalue()

    manager._safe_archive(archive("workspace/file.txt"))
    manager._safe_archive(archive("workspace/link", "../other/file"))
    for bad in ("/etc/passwd", "../../etc/passwd", "workspace/link"):
        with pytest.raises(ValueError, match="Unsafe"):
            manager._safe_archive(archive(bad, "../../etc/passwd" if bad == "workspace/link" else None))


@pytest.mark.asyncio
async def test_lifecycle_lock_rejects_invalid_ids_before_creating_state(monkeypatch):
    monkeypatch.setattr(manager, "LOCKS", {})
    with pytest.raises(ValueError):
        async with manager._exclusive("../escape"):
            pass
    assert manager.LOCKS == {}


@pytest.mark.asyncio
async def test_lifecycle_lock_returns_conflict_for_concurrent_calls(monkeypatch):
    monkeypatch.setattr(manager, "LOCKS", {})
    async with manager._exclusive("a" * 8):
        with pytest.raises(manager.HTTPException) as caught:
            async with manager._exclusive("a" * 8):
                pass
        assert caught.value.status_code == 409
