import json

import pytest
from mix_agent.mcp import protocol
from mix_agent.tools import network


async def test_mcp_call_uses_pinned_public_request(monkeypatch):
    captured = {}

    async def request(method, url, **kwargs):
        captured.update(method=method, url=url, **kwargs)
        return 200, {"content-type": "application/json"}, b'{"result":{"tools":[]}}'

    monkeypatch.setattr(protocol, "request_public", request)
    assert await protocol.modern_call("https://example.com/mcp", "tools/list") == {"tools": []}
    assert captured["method"] == "POST"
    assert captured["url"] == "https://example.com/mcp"
    assert captured["json"]["method"] == "tools/list"


async def test_public_request_blocks_private_dns_before_connection(monkeypatch):
    async def private_address(host, port):
        raise ValueError("Private, loopback and metadata addresses are blocked")

    class UnexpectedClient:
        def __init__(self, **kwargs):
            pytest.fail("A blocked address must not open a client")

    monkeypatch.setattr(network, "public_address", private_address)
    monkeypatch.setattr(network.httpx, "AsyncClient", UnexpectedClient)
    with pytest.raises(ValueError, match="Private"):
        await network.request_public("POST", "https://example.com/mcp", json={})


async def test_mcp_rejects_redirect_without_following_it(monkeypatch):
    async def request(*args, **kwargs):
        return 302, {"location": "https://internal.example/mcp"}, b""

    monkeypatch.setattr(protocol, "request_public", request)
    with pytest.raises(ValueError, match="MCP operation failed"):
        await protocol.modern_call("https://example.com/mcp", "tools/list")


async def test_modern_path_reaches_the_egress_proxy_end_to_end(monkeypatch):
    """The internal MCP runner cannot dial the internet directly.

    Its explicit MIX_EGRESS_PROXY must be honoured (the runner maps its
    Compose-provided HTTPS_PROXY into it at startup), the URL must keep the
    real hostname for CONNECT, and the legacy 115s timeout must not hold a
    worker hostage.
    """
    captured = {}

    class Response:
        status_code = 200
        headers = {"content-type": "application/json"}

        async def aiter_bytes(self):
            yield b'{"result":{"tools":[{"name":"echo"}]}}'

    class Stream:
        async def __aenter__(self):
            return Response()

        async def __aexit__(self, *_exc):
            return False

    class Client:
        def __init__(self, **kwargs):
            captured.update(client=kwargs)

        async def __aenter__(self):
            return self

        async def __aexit__(self, *_exc):
            return False

        def stream(self, method, url, **kwargs):
            captured.update(method=method, url=url, **kwargs)
            return Stream()

    async def address(_host, _port):
        return "93.184.216.34"

    monkeypatch.setattr(network, "public_address", address)
    monkeypatch.setattr(network.httpx, "AsyncClient", Client)
    monkeypatch.setenv("MIX_EGRESS_PROXY", "http://egress-proxy:3128")

    result = await protocol.modern_call("https://mcp.example.com/mcp", "tools/list")
    assert result == {"tools": [{"name": "echo"}]}
    assert captured["client"]["proxy"] == "http://egress-proxy:3128"
    assert captured["client"]["trust_env"] is False
    assert captured["url"] == "https://mcp.example.com/mcp"
    assert captured["method"] == "POST"
    assert captured["headers"]["Mcp-Method"] == "tools/list"
    assert captured["headers"]["MCP-Protocol-Version"] == protocol.MODERN_VERSION
    assert captured["client"]["timeout"] <= 30


async def test_request_public_enforces_explicit_allowed_domains(monkeypatch):
    async def address(_host, _port):
        pytest.fail("DNS must not be queried for a denied host")

    monkeypatch.setattr(network, "public_address", address)
    with pytest.raises(ValueError, match="allowed list"):
        await network.request_public("POST", "https://blocked.example", allowed_domains=["example.com"])


async def test_request_public_honours_global_none_mode(tmp_path, monkeypatch):
    policy = tmp_path / "policy.json"
    policy.write_text(json.dumps({"mode": "none", "allowed_domains": []}))
    monkeypatch.setenv("MIX_EGRESS_POLICY_FILE", str(policy))
    with pytest.raises(ValueError, match="Egress is disabled"):
        await network.request_public("POST", "https://example.com/mcp")


def test_allow_list_rejects_malformed_entries():
    for bad in ("foo..bar", "-bad.example", "1.2.3.4", "good.example-", "localhost", "metadata"):
        with pytest.raises(ValueError, match="Invalid allowed domain"):
            network.validate_domain(bad)
    assert network.validate_domain("Good.Example.") == "good.example"


async def test_generic_https_proxy_is_not_implicitly_used(monkeypatch):
    """Only MIX_EGRESS_PROXY opts a caller into the shared egress proxy."""
    captured = {}

    async def address(_host, _port):
        return "93.184.216.34"

    class Response:
        status_code = 200
        headers = {}

        async def aiter_bytes(self):
            yield b"{}"

    class Stream:
        async def __aenter__(self):
            return Response()

        async def __aexit__(self, *_exc):
            return False

    class Client:
        def __init__(self, **kwargs):
            captured.update(kwargs)

        async def __aenter__(self):
            return self

        async def __aexit__(self, *_exc):
            return False

        def stream(self, *_args, **_kwargs):
            return Stream()

    monkeypatch.setattr(network, "public_address", address)
    monkeypatch.setattr(network.httpx, "AsyncClient", Client)
    monkeypatch.setenv("HTTPS_PROXY", "http://attacker-proxy:3128")
    monkeypatch.delenv("MIX_EGRESS_PROXY", raising=False)
    await network.request_public("GET", "https://example.com/x")
    assert captured["proxy"] is None


async def test_fetch_public_bytes_uses_the_explicit_proxy(monkeypatch):
    """web_fetch/oauth metadata must reach the same proxy as request_public."""
    captured = {}

    async def address(_host, _port):
        return "93.184.216.34"

    class Response:
        is_redirect = False
        headers = {"content-type": "text/plain"}

        def raise_for_status(self):
            pass

        async def aiter_bytes(self):
            yield b"hello"

    class Stream:
        async def __aenter__(self):
            return Response()

        async def __aexit__(self, *_exc):
            return False

    class Client:
        def __init__(self, **kwargs):
            captured.update(client=kwargs)

        async def __aenter__(self):
            return self

        async def __aexit__(self, *_exc):
            return False

        def stream(self, method, url, **kwargs):
            captured.update(method=method, url=url, **kwargs)
            return Stream()

    monkeypatch.setattr(network, "public_address", address)
    monkeypatch.setattr(network.httpx, "AsyncClient", Client)
    monkeypatch.setenv("MIX_EGRESS_PROXY", "http://egress-proxy:3128")
    body, content_type = await network.fetch_public_bytes("https://mcp.example.com/file.txt")
    assert body == b"hello"
    assert captured["client"]["proxy"] == "http://egress-proxy:3128"
    # With a proxy the real hostname is kept for CONNECT, not the pinned IP.
    assert captured["url"] == "https://mcp.example.com/file.txt"
    assert captured["headers"] == {}
