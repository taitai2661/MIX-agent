"""Official MCP Registry client and untrusted manifest normalization."""

from __future__ import annotations

import asyncio
import re
import time
from copy import deepcopy
from urllib.parse import quote, urlsplit

import httpx

REGISTRY_ORIGIN = "https://registry.modelcontextprotocol.io"
SUPPORTED_PACKAGES = ("oci", "npm", "pypi")
PACKAGE_PRIORITY = {name: index for index, name in enumerate(SUPPORTED_PACKAGES)}
ALLOWED_REGISTRIES = {
    "npm": {"", "https://registry.npmjs.org"},
    "pypi": {"", "https://pypi.org", "https://pypi.org/simple"},
}
OCI_HOSTS = {"docker.io", "ghcr.io", "quay.io", "mcr.microsoft.com"}
VERSION_RE = re.compile(r"^[0-9A-Za-z][0-9A-Za-z._+-]{0,254}$")
_CACHE: dict[str, tuple[float, dict]] = {}
_CACHE_MAX = 200
_LOCK = asyncio.Lock()


def _exact_version(value: object) -> str:
    version = str(value or "")
    if not VERSION_RE.fullmatch(version) or version == "latest" or any(c in version for c in "*<>=~^|"):
        raise ValueError("Registry package must use an exact version")
    return version


def _input(item: dict) -> dict:
    return {
        "name": str(item.get("name") or item.get("valueHint") or "")[:160],
        "description": str(item.get("description") or "")[:1000],
        "required": bool(item.get("isRequired")),
        "secret": bool(item.get("isSecret")),
        "default": item.get("default") if not item.get("isSecret") else None,
    }


def normalize(server_response: dict) -> dict:
    """Convert registry data into a bounded, executable-neutral manifest."""
    server = server_response.get("server", server_response)
    if not isinstance(server, dict):
        raise ValueError("Invalid Registry response")
    name = str(server.get("name") or "")
    if not name or len(name) > 255:
        raise ValueError("Invalid Registry server name")
    version = _exact_version(server.get("version"))
    remotes = []
    for remote in server.get("remotes") or []:
        transport = remote.get("transport", remote)
        url = str(transport.get("url") or "")
        parsed = urlsplit(url)
        if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
            continue
        remotes.append({
            "kind": "remote",
            "transport": str(transport.get("type") or "streamable-http"),
            "url": url,
            "headers": [_input(value) for value in transport.get("headers") or []],
            "variables": {key: _input({"name": key, **value}) for key, value in (transport.get("variables") or {}).items()},
        })
    packages = []
    unsupported = []
    for package in server.get("packages") or []:
        kind = str(package.get("registryType") or "").lower()
        if kind not in SUPPORTED_PACKAGES:
            unsupported.append(kind or "unknown")
            continue
        identifier = str(package.get("identifier") or "")
        package_version = _exact_version(package.get("version") or version)
        registry_base = str(package.get("registryBaseUrl") or "")
        if kind in ALLOWED_REGISTRIES and registry_base.rstrip("/") not in {v.rstrip("/") for v in ALLOWED_REGISTRIES[kind]}:
            continue
        if kind == "oci":
            host = identifier.split("/", 1)[0].lower()
            if not (host in OCI_HOSTS or host.endswith(".pkg.dev") or host.endswith(".azurecr.io")):
                continue
            if "@sha256:" not in identifier and ":" not in identifier.rsplit("/", 1)[-1]:
                continue
        elif not identifier or any(ch.isspace() for ch in identifier):
            continue
        transport = package.get("transport") or {}
        if transport.get("type", "stdio") != "stdio":
            continue
        packages.append({
            "kind": kind,
            "identifier": identifier,
            "version": package_version,
            "registry_base_url": registry_base,
            "runtime_hint": str(package.get("runtimeHint") or ""),
            "runtime_arguments": [_input(value) for value in package.get("runtimeArguments") or []],
            "package_arguments": [_input(value) for value in package.get("packageArguments") or []],
            "environment": [_input(value) for value in package.get("environmentVariables") or []],
        })
    packages.sort(key=lambda item: PACKAGE_PRIORITY[item["kind"]])
    candidates = remotes + packages
    selected = remotes[0] if remotes else (packages[0] if packages else None)
    return {
        "schema_version": 1,
        "registry_id": name,
        "title": str(server.get("title") or name)[:200],
        "description": str(server.get("description") or "")[:4000],
        "version": version,
        "repository": deepcopy(server.get("repository") or {}),
        "candidates": candidates,
        "selected": selected,
        "unsupported_formats": sorted(set(unsupported)),
    }


async def _get(path: str, params: dict | None = None) -> dict:
    key = path + "?" + repr(sorted((params or {}).items()))
    async with _LOCK:
        cached = _CACHE.get(key)
    try:
        async with httpx.AsyncClient(timeout=15, trust_env=False) as client:
            response = await client.get(REGISTRY_ORIGIN + path, params=params)
            response.raise_for_status()
            value = response.json()
        async with _LOCK:
            _CACHE[key] = (time.monotonic(), value)
            if len(_CACHE) > _CACHE_MAX:
                oldest = min(_CACHE, key=lambda k: _CACHE[k][0])
                del _CACHE[oldest]
        return value
    except Exception:
        if cached and time.monotonic() - cached[0] < 3600:
            return deepcopy(cached[1])
        raise


async def search(query: str = "", cursor: str = "", limit: int = 30) -> dict:
    params = {"limit": max(1, min(limit, 50)), "version": "latest"}
    if query:
        params["search"] = query[:200]
    if cursor:
        params["cursor"] = cursor[:1000]
    raw = await _get("/v0.1/servers", params)
    rows = []
    for item in raw.get("servers", []):
        try:
            rows.append(normalize(item))
        except ValueError:
            continue
    return {"servers": rows, "metadata": raw.get("metadata", {})}


async def detail(name: str, version: str = "latest") -> dict:
    if not name or len(name) > 255:
        raise ValueError("Invalid Registry server name")
    raw = await _get(f"/v0.1/servers/{quote(name, safe='')}/versions/{quote(version, safe='')}")
    return normalize(raw)

