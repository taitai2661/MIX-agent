import importlib.util
from pathlib import Path
import pytest
from cryptography.exceptions import InvalidTag
from mix_agent.auth.security import encrypt, decrypt
from mix_agent.storage.backup import seal, unseal, validate
from mix_agent.tools import network
from mix_agent.tools.network import public_address, fetch_public


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


ROOT = Path(__file__).resolve().parents[2]
runner = module("runner_test", ROOT / "runners/execution/main.py")
snapshots = module("snapshot_test", ROOT / "packages/tool_contracts/snapshots.py")


def test_encryption_purpose_is_authenticated():
    item = encrypt("not-a-real-key", "provider")
    assert "not-a-real-key" not in str(item)
    assert decrypt(item) == "not-a-real-key"
    item["purpose"] = "other"
    with pytest.raises(InvalidTag):
        decrypt(item)


def test_backup_tampering_and_password():
    blob = seal(b'{"test":1}', "backup-passphrase")
    assert unseal(blob, "backup-passphrase") == b'{"test":1}'
    with pytest.raises(InvalidTag):
        unseal(blob, "wrong-password")
    with pytest.raises(InvalidTag):
        unseal(blob[:-1] + bytes([blob[-1] ^ 1]), "backup-passphrase")


@pytest.mark.parametrize(
    "path", ["../secret", "/etc/passwd", "/workspace/../keys", "/var/run/docker.sock"]
)
def test_path_escape(path):
    with pytest.raises(ValueError):
        runner.parts(path)


def test_symlink_read_and_parent_write_blocked(tmp_path, monkeypatch):
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    monkeypatch.setattr(runner, "ROOT", workspace)
    private = tmp_path / "private"
    private.mkdir()
    (private / "secret").write_text("must not be read")
    (workspace / "link").symlink_to(private, target_is_directory=True)
    with pytest.raises(OSError):
        runner.read("link/secret")
    with pytest.raises(OSError):
        runner.write("link/secret", "overwrite")
    assert (private / "secret").read_text() == "must not be read"
    runner.write("notes/hello.txt", "hello")
    assert runner.read("notes/hello.txt") == "hello"


@pytest.mark.parametrize(
    "name", ["../escape", "/etc/passwd", "x/../../escape", "x\\escape"]
)
def test_backup_path_traversal(name):
    with pytest.raises(ValueError):
        snapshots.validate_snapshot({"files": {name: "aGk="}})


def test_snapshot_roundtrip(tmp_path):
    (tmp_path / "example.txt").write_text("original")
    old = snapshots.capture(tmp_path)
    snapshots.restore(tmp_path, {"files": {"nested/file.txt": "aGVsbG8="}})
    assert (tmp_path / "nested/file.txt").read_text() == "hello"
    assert not (tmp_path / "example.txt").exists()
    snapshots.restore(tmp_path, old)
    assert (tmp_path / "example.txt").read_text() == "original"


@pytest.mark.parametrize("host", ["127.0.0.1", "::1", "169.254.169.254", "10.0.0.1"])
async def test_private_network_blocked(host):
    with pytest.raises(ValueError):
        await public_address(host, 80)


async def test_non_http_fetch_blocked():
    with pytest.raises(ValueError):
        await fetch_public("file:///etc/passwd")


async def test_public_fetch_passes_string_sni_hostname(monkeypatch):
    captured = {}

    class Response:
        is_redirect = False

        def raise_for_status(self):
            pass

        async def aiter_bytes(self):
            yield b"page"

    class Stream:
        async def __aenter__(self):
            return Response()

        async def __aexit__(self, *args):
            pass

    class Client:
        def __init__(self, **kwargs):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *args):
            pass

        def stream(self, method, url, **kwargs):
            captured.update(kwargs)
            return Stream()

    async def address(host, port):
        return "93.184.216.34"

    monkeypatch.setattr(network, "public_address", address)
    monkeypatch.setattr(network.httpx, "AsyncClient", Client)

    assert await fetch_public("https://example.com/page") == "page"
    assert captured["extensions"]["sni_hostname"] == "example.com"
    assert isinstance(captured["extensions"]["sni_hostname"], str)


def test_setup_only_once_and_csrf(signed):
    assert (
        signed.post(
            "/api/v1/setup/admin",
            json={"username": "other", "password": "other-password-123"},
        ).status_code
        == 409
    )
    assert (
        signed.post(
            "/api/v1/conversations", json={}, headers={"x-csrf-token": "wrong"}
        ).status_code
        == 403
    )
    assert (
        signed.post(
            "/api/v1/conversations", json={}, headers={"origin": "https://evil.example"}
        ).status_code
        == 403
    )
    assert signed.post("/api/v1/conversations", json={}).status_code == 200


def test_direct_private_origin_allows_setup_and_followup_settings(client):
    headers = {"host": "192.168.1.24:8080", "origin": "http://192.168.1.24:8080"}
    response = client.post(
        "/api/v1/setup/admin",
        json={"username": "lan-admin", "password": "lan-password-12345"},
        headers=headers,
    )
    assert response.status_code == 200, response.text
    response = client.put(
        "/api/v1/settings",
        json={"setup_complete": True},
        headers={**headers, "x-csrf-token": response.json()["csrf"]},
    )
    assert response.status_code == 200, response.text


@pytest.mark.parametrize(
    "headers",
    [
        {"host": "192.168.1.24:8080", "origin": "http://192.168.1.25:8080"},
        {"host": "192.168.1.24:8080", "origin": "http://192.168.1.24:8081"},
        {"host": "192.168.1.24:8080", "origin": "https://192.168.1.24:8080"},
        {"host": "8.8.8.8:8080", "origin": "http://8.8.8.8:8080"},
        {"host": "192.168.1.24:8080", "origin": "http://192.168.1.24:8080", "x-forwarded-host": "example.test"},
    ],
)
def test_direct_origin_rejects_mismatched_public_or_forwarded_hosts(client, headers):
    response = client.post(
        "/api/v1/setup/admin",
        json={"username": "blocked", "password": "blocked-password-12345"},
        headers=headers,
    )
    assert response.status_code == 403


def test_unauthorized_and_validation_redaction(client):
    assert client.get("/api/v1/providers").status_code == 401
    result = client.post(
        "/api/v1/setup/admin", json={"username": "x", "password": "secret"}
    )
    assert result.status_code == 422
    assert '"input"' not in result.text
    assert "secret" not in result.text
