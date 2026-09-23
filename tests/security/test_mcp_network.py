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
