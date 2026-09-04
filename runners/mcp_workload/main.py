"""Generic npm/PyPI MCP workload. Package code and secrets never run in MIX app."""

from __future__ import annotations

import asyncio
import json
import os
from contextlib import AsyncExitStack
from pathlib import Path

from fastapi import FastAPI, HTTPException
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

app = FastAPI(docs_url=None, redoc_url=None)
ROOT = Path("/data/.packages")
LOCK = asyncio.Lock()
SESSION = None
STACK = None
PROCESS_KEY = ""


async def _install() -> list[str]:
    kind = os.environ["MCP_PACKAGE_KIND"]
    identifier = os.environ["MCP_PACKAGE_IDENTIFIER"]
    version = os.environ["MCP_PACKAGE_VERSION"]
    ROOT.mkdir(parents=True, exist_ok=True)
    marker = ROOT / "manifest.json"
    wanted = {"kind": kind, "identifier": identifier, "version": version}
    if not marker.exists() or json.loads(marker.read_text()) != wanted:
        if kind == "npm":
            command = ["npm", "install", "--prefix", str(ROOT), "--save-exact", f"{identifier}@{version}"]
        elif kind == "pypi":
            command = ["python", "-m", "pip", "install", "--no-cache-dir", "--prefix", str(ROOT), f"{identifier}=={version}"]
        else:
            raise ValueError("Unsupported generic runtime package")
        process = await asyncio.create_subprocess_exec(*command, stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.DEVNULL)
        if await asyncio.wait_for(process.wait(), 300):
            raise ValueError("MCP package installation failed")
        marker.write_text(json.dumps(wanted, sort_keys=True))
    if kind == "npm":
        bins = sorted((ROOT / "node_modules" / ".bin").iterdir())
    else:
        bins = sorted((ROOT / "bin").iterdir())
    bins = [path for path in bins if path.is_file() or path.is_symlink()]
    if not bins:
        raise ValueError("MCP package has no executable")
    preferred = identifier.rsplit("/", 1)[-1].replace("_", "-")
    if kind == "npm":
        package = json.loads((ROOT / "node_modules" / identifier / "package.json").read_text())
        declared = package.get("bin", {})
        if isinstance(declared, str):
            preferred = package.get("name", preferred).rsplit("/", 1)[-1]
        elif declared:
            preferred = next(iter(declared))
    executable = next((path for path in bins if path.name == preferred), bins[0])
    return [str(executable)]


async def _session(credentials: dict, arguments: list[str]):
    global SESSION, STACK, PROCESS_KEY
    key = json.dumps({"credentials": credentials, "arguments": arguments}, sort_keys=True)
    if SESSION is not None and PROCESS_KEY == key:
        return SESSION
    if STACK is not None:
        try:
            await STACK.aclose()
        except BaseException:
            pass
        SESSION = None
        STACK = None
    executable = await _install()
    env = {key: value for key, value in os.environ.items() if key in ("PATH", "HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy")}
    custom = credentials.get("env", {})
    if any(key in custom for key in ("PATH", "HOME", "TMPDIR", "HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "NO_PROXY", "no_proxy")):
        raise ValueError("Reserved runtime environment variable")
    env.update(custom)
    env.update(HOME="/data", TMPDIR="/tmp")
    python_sites = list(ROOT.glob("lib/python*/site-packages"))
    if python_sites:
        env["PYTHONPATH"] = str(python_sites[0])
    stack = AsyncExitStack()
    params = StdioServerParameters(command=executable[0], args=arguments, env=env, cwd="/data")
    errlog = stack.enter_context(open(os.devnull, "w"))
    read, write = await stack.enter_async_context(stdio_client(params, errlog=errlog))
    session = await stack.enter_async_context(ClientSession(read, write))
    await session.initialize()
    STACK, SESSION, PROCESS_KEY = stack, session, key
    return session


@app.post("/discover")
async def discover(body: dict):
    async with LOCK:
        try:
            session = await _session(body.get("credentials", {}), body.get("arguments", []))
            result = await asyncio.wait_for(session.list_tools(), 30)
            return {"tools": [tool.model_dump(mode="json", by_alias=True, exclude_none=True) for tool in result.tools]}
        except Exception:
            raise HTTPException(502, "MCP discovery failed")


@app.post("/call")
async def call(body: dict):
    async with LOCK:
        try:
            session = await _session(body.get("credentials", {}), body.get("arguments", []))
            result = await asyncio.wait_for(session.call_tool(body["tool"], body.get("arguments_value", {})), 110)
            return result.model_dump(mode="json", by_alias=True, exclude_none=True)
        except Exception:
            raise HTTPException(502, "MCP call failed")
