"""Proxy tunnels must keep streaming after the client half-closes its side."""

import asyncio
import json

import pytest
from deploy.egress import proxy


@pytest.mark.asyncio
async def test_half_closed_client_still_receives_the_response(tmp_path, monkeypatch):
    policy = tmp_path / "policy.json"
    policy.write_text(json.dumps({"mode": "public_web", "allowed_domains": []}))
    monkeypatch.setattr(proxy, "POLICY", policy)
    monkeypatch.setattr(proxy, "MCP_POLICY", tmp_path / "mcp-policy.json")

    async def fake_public_address(host, port):
        return "127.0.0.1"

    monkeypatch.setattr(proxy, "public_address", fake_public_address)

    async def upstream(reader, writer):
        await reader.read(65536)
        # Give the client's FIN time to reach the proxy before answering.
        await asyncio.sleep(0.05)
        writer.write(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
        await writer.drain()
        writer.close()

    upstream_server = await asyncio.start_server(upstream, "127.0.0.1", 0)
    upstream_port = upstream_server.sockets[0].getsockname()[1]
    real_open = asyncio.open_connection

    async def open_upstream(host, port, **kwargs):
        return await real_open("127.0.0.1", upstream_port, **kwargs)

    monkeypatch.setattr(proxy.asyncio, "open_connection", open_upstream)
    proxy_server = await asyncio.start_server(proxy.handle, "127.0.0.1", 0)
    proxy_port = proxy_server.sockets[0].getsockname()[1]

    try:
        reader, writer = await real_open("127.0.0.1", proxy_port)
        writer.write(b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n")
        await writer.drain()
        writer.write_eof()  # done sending, still waiting for the answer
        body = await asyncio.wait_for(reader.read(), 5)
        assert body.endswith(b"hello")
        writer.close()
    finally:
        proxy_server.close()
        upstream_server.close()
        await asyncio.gather(proxy_server.wait_closed(), upstream_server.wait_closed())
