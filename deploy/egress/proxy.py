"""Minimal HTTP CONNECT gateway. Exact-host ACL, public-IP validation and DNS pinning."""

import asyncio
import json
import os
from pathlib import Path
from urllib.parse import urlsplit

from mix_agent.tools.network import public_address

POLICY = Path(os.getenv("POLICY_FILE", "/policy/policy.json"))
MCP_POLICY = Path(os.getenv("MCP_POLICY_FILE", "/policy/mcp-policy.json"))


def host_allowed(host, allowed_domains):
    """An empty list means unrestricted public egress; otherwise use exact matches."""
    return not allowed_domains or host in allowed_domains


async def transfer(reader, writer):
    try:
        while chunk := await asyncio.wait_for(reader.read(65536), 120):
            writer.write(chunk)
            await writer.drain()
    finally:
        writer.close()


async def handle(reader, writer):
    upstream = None
    try:
        header = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 10)
        if len(header) > 16384:
            raise ValueError()
        first, *lines = header.decode("latin1").split("\r\n")
        method, target, version = first.split(" ")
        if method == "CONNECT":
            host, port = target.rsplit(":", 1)
            host, port = host.lower(), int(port)
        else:
            parsed = urlsplit(target)
            if parsed.scheme != "http" or parsed.username or parsed.password:
                raise ValueError()
            host, port = parsed.hostname.lower(), parsed.port or 80
        policy = json.loads(POLICY.read_text()) if POLICY.exists() else {}
        source = writer.get_extra_info("peername")
        source_ip = source[0] if source else ""
        mcp_policy = json.loads(MCP_POLICY.read_text()) if MCP_POLICY.exists() else {}
        source_policy = mcp_policy.get("source_policies", {}).get(source_ip)
        mode = source_policy.get("mode") if source_policy else "public_web"
        allowed_domains = source_policy.get("allowed_domains", []) if source_policy else policy.get("allowed_domains", [])
        if mode == "none" or not host_allowed(host, allowed_domains) or port not in (80, 443):
            raise ValueError()
        ip = await public_address(host, port)
        remote, upstream = await asyncio.wait_for(asyncio.open_connection(ip, port), 10)
        if method == "CONNECT":
            writer.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        else:
            path = parsed.path or "/"
            if parsed.query:
                path += "?" + parsed.query
            safe = [
                line
                for line in lines
                if line
                and line.split(":", 1)[0].lower()
                not in ("host", "proxy-authorization", "proxy-connection", "connection")
            ]
            upstream.write(
                (
                    f"{method} {path} {version}\r\nHost: {host}\r\nConnection: close\r\n"
                    + "\r\n".join(safe)
                    + "\r\n\r\n"
                ).encode("latin1")
            )
        await writer.drain()
        tasks = [
            asyncio.create_task(transfer(reader, upstream)),
            asyncio.create_task(transfer(remote, writer)),
        ]
        _, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        for task in pending:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
    except Exception:
        if not writer.is_closing():
            writer.write(
                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
    finally:
        writer.close()
        if upstream:
            upstream.close()


async def main():
    server = await asyncio.start_server(handle, "0.0.0.0", 3128, limit=16384)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
