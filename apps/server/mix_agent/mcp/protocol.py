"""MCP 2026-07-28 stateless HTTP client with an explicit legacy boundary."""

from __future__ import annotations

import itertools
from urllib.parse import urlsplit

import httpx

MODERN_VERSION = "2026-07-28"
LEGACY_VERSION = "2025-11-25"
_IDS = itertools.count(1)


def validate_schema(schema: object, *, depth: int = 0, nodes: list[int] | None = None) -> None:
    nodes = nodes or [0]
    nodes[0] += 1
    if nodes[0] > 2000 or depth > 32:
        raise ValueError("MCP tool schema exceeds safety limits")
    if isinstance(schema, dict):
        for key, value in schema.items():
            if key == "$ref" and isinstance(value, str) and not value.startswith("#"):
                raise ValueError("External MCP schema references are disabled")
            validate_schema(value, depth=depth + 1, nodes=nodes)
    elif isinstance(schema, list):
        for value in schema:
            validate_schema(value, depth=depth + 1, nodes=nodes)


def _headers(method: str, tool: str = "", headers: dict | None = None) -> dict:
    result = {
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
        "MCP-Protocol-Version": MODERN_VERSION,
        "Mcp-Method": method,
    }
    if tool:
        result["Mcp-Name"] = tool
    result.update(headers or {})
    return result


def _request(method: str, params: dict | None = None) -> dict:
    params = dict(params or {})
    meta = dict(params.get("_meta") or {})
    meta["io.modelcontextprotocol/clientInfo"] = {"name": "MIX agent", "version": "0.1.0"}
    meta["io.modelcontextprotocol/protocolVersion"] = MODERN_VERSION
    params["_meta"] = meta
    return {"jsonrpc": "2.0", "id": next(_IDS), "method": method, "params": params}


async def modern_call(url: str, method: str, params: dict | None = None, *, tool: str = "", headers: dict | None = None) -> dict:
    parsed = urlsplit(url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError("Remote MCP requires a public HTTPS URL")
    import os
    async with httpx.AsyncClient(timeout=115, follow_redirects=False, trust_env=False, proxy=os.getenv("HTTPS_PROXY")) as client:
        response = await client.post(url, headers=_headers(method, tool, headers), json=_request(method, params))
        response.raise_for_status()
        if "text/event-stream" in response.headers.get("content-type", ""):
            raise ValueError("Modern MCP must return a bounded request response")
        value = response.json()
    if value.get("error"):
        raise ValueError("MCP operation failed")
    return value.get("result", value)


async def discover(url: str, headers: dict | None = None) -> dict:
    try:
        return await modern_call(url, "server/discover", headers=headers)
    except Exception:
        return await modern_call(url, "tools/list", headers=headers)


async def call(url: str, tool: str, arguments: dict, headers: dict | None = None) -> dict:
    return await modern_call(url, "tools/call", {"name": tool, "arguments": arguments}, tool=tool, headers=headers)
