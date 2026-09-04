import asyncio
import hashlib
import json
import os
import secrets
from contextlib import AsyncExitStack
from pathlib import Path

import httpx
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client
from mcp.client.streamable_http import streamablehttp_client

try:
    from mcp_protocol import call as modern_call
    from mcp_protocol import discover as modern_discover
except ImportError:  # Source-tree tests.
    from mix_agent.mcp.protocol import call as modern_call
    from mix_agent.mcp.protocol import discover as modern_discover

app = FastAPI(docs_url=None, redoc_url=None)
CONNECTIONS = {}
RUN_TASKS = {}
SHARED = Path(os.getenv("MCP_SHARED", "/shared"))


@app.middleware("http")
async def auth(request: Request, call_next):
    token = Path(os.getenv("TOKEN_FILE", "/tokens/mcp.token")).read_text().strip()
    if not secrets.compare_digest(
        request.headers.get("authorization", ""), "Bearer " + token
    ):
        return JSONResponse({"detail": "Unauthorized"}, 401)
    return await call_next(request)


async def serve(connection, queue):
    try:
        async with AsyncExitStack() as stack:
            credentials = connection.get("credentials", {})
            if connection["transport"] == "stdio":
                command = connection["command"]
                # No package fetch on tool invocation. Provisioning is a separate admin action.
                if command in ("npx", "uvx"):
                    raise ValueError(
                        "Use an installed executable; automatic downloads are disabled"
                    )
                env = {
                    k: v
                    for k, v in os.environ.items()
                    if k
                    in (
                        "PATH",
                        "HTTP_PROXY",
                        "HTTPS_PROXY",
                        "http_proxy",
                        "https_proxy",
                    )
                }
                env.update(HOME="/packages", TMPDIR="/tmp")
                custom_env = credentials.get("env", {})
                if any(
                    k
                    in (
                        "HTTP_PROXY",
                        "HTTPS_PROXY",
                        "http_proxy",
                        "https_proxy",
                        "NO_PROXY",
                        "no_proxy",
                        "PATH",
                    )
                    for k in custom_env
                ):
                    raise ValueError("Proxy and PATH settings cannot be overridden")
                env.update(custom_env)
                params = StdioServerParameters(
                    command=command,
                    args=connection.get("args", []),
                    env=env,
                    cwd=str(SHARED),
                )
                errlog = stack.enter_context(open(os.devnull, "w"))
                read, write = await stack.enter_async_context(
                    stdio_client(params, errlog=errlog)
                )
            else:
                from urllib.parse import urlsplit

                parsed = urlsplit(connection["url"])
                if (
                    parsed.scheme != "https"
                    or not parsed.hostname
                    or parsed.username
                    or parsed.password
                ):
                    raise ValueError(
                        "Remote MCP requires HTTPS; private endpoints are disabled"
                    )
                headers = credentials.get("headers", {})
                if any(k.lower() in ("host", "proxy-authorization") for k in headers):
                    raise ValueError("Reserved header")

                def factory(headers=None, timeout=None, auth=None):
                    return httpx.AsyncClient(
                        headers=headers,
                        timeout=timeout or 30,
                        auth=auth,
                        follow_redirects=False,
                        proxy=os.getenv("HTTPS_PROXY", "http://egress-proxy:3128"),
                        trust_env=False,
                    )

                read, write, _ = await stack.enter_async_context(
                    streamablehttp_client(
                        connection["url"], headers=headers, httpx_client_factory=factory
                    )
                )
            session = await stack.enter_async_context(ClientSession(read, write))
            await session.initialize()
            while True:
                action, body, future = await queue.get()
                if future.cancelled():
                    continue
                try:
                    if action == "discover":
                        result = await asyncio.wait_for(session.list_tools(), 30)
                        value = {
                            "tools": [
                                t.model_dump(
                                    mode="json", by_alias=True, exclude_none=True
                                )
                                for t in result.tools
                            ]
                        }
                    else:
                        result = await asyncio.wait_for(
                            session.call_tool(body["tool"], body.get("arguments", {})),
                            110,
                        )
                        value = result.model_dump(
                            mode="json", by_alias=True, exclude_none=True
                        )
                    if not future.done():
                        future.set_result(value)
                except Exception:
                    if not future.done():
                        future.set_exception(ValueError("MCP operation failed"))
    except BaseException:
        while not queue.empty():
            _, _, future = queue.get_nowait()
            if not future.done():
                future.set_exception(ValueError("MCP connection failed"))


async def invoke(action, body):
    connection = body["connection"]
    if connection.get("transport") == "http" and connection.get("protocol_generation") == "2026-07-28":
        credentials = connection.get("credentials", {})
        headers = credentials.get("headers", {})
        try:
            if action == "discover":
                result = await modern_discover(connection["url"], headers)
                tools = result.get("tools", result.get("capabilities", {}).get("tools", []))
                return {"tools": tools, "protocol_generation": "2026-07-28", "cache": {key: result.get(key) for key in ("ttlMs", "cacheScope") if key in result}}
            return await modern_call(connection["url"], body["tool"], body.get("arguments", {}), headers)
        except httpx.HTTPStatusError as error:
            if error.response.status_code in (401, 403):
                raise HTTPException(error.response.status_code, "MCP authorization is required")
            if not connection.get("allow_legacy", True):
                raise HTTPException(502, "MCP 2026-07-28 operation failed")
            # Compatibility adapter below owns all initialize/session behavior.
        except Exception:
            if not connection.get("allow_legacy", True):
                raise HTTPException(502, "MCP 2026-07-28 operation failed")
            # Compatibility adapter below owns all initialize/session behavior.
    key = hashlib.sha256(json.dumps(connection, sort_keys=True).encode()).hexdigest()
    if key not in CONNECTIONS or CONNECTIONS[key][1].done():
        queue = asyncio.Queue()
        CONNECTIONS[key] = (queue, asyncio.create_task(serve(connection, queue)))
    queue, task = CONNECTIONS[key]
    future = asyncio.get_running_loop().create_future()
    await queue.put((action, body, future))
    if body.get("run_id"):
        RUN_TASKS[body["run_id"]] = (future, task)
    try:
        return await asyncio.wait_for(future, 115)
    except Exception:
        raise HTTPException(502, "MCP接続またはTool実行に失敗しました")
    finally:
        RUN_TASKS.pop(body.get("run_id"), None)


@app.post("/discover")
async def discover(body: dict):
    return await invoke("discover", body)


@app.post("/call")
async def call(body: dict):
    return await invoke("call", body)


@app.post("/cancel")
async def cancel(body: dict):
    if pair := RUN_TASKS.get(body["run_id"]):
        pair[0].cancel()
        pair[1].cancel()
    return {"ok": True, "remote_side_effects_may_continue": True}


@app.post("/install")
async def install(body: dict):
    # The only initial installer is the pinned Filesystem template.
    connection = body["connection"]
    if connection.get("command") != "/packages/node_modules/.bin/mcp-server-filesystem":
        raise HTTPException(422, "初期版の自動導入はFilesystemテンプレートのみです")
    process = await asyncio.create_subprocess_exec(
        "npm",
        "install",
        "--prefix",
        "/packages",
        "--ignore-scripts",
        "--save-exact",
        "@modelcontextprotocol/server-filesystem@2025.8.21",
        stdout=asyncio.subprocess.DEVNULL,
        stderr=asyncio.subprocess.DEVNULL,
    )
    try:
        code = await asyncio.wait_for(process.wait(), 100)
    except TimeoutError:
        process.kill()
        await process.wait()
        raise HTTPException(504, "Package installation timed out")
    if code:
        raise HTTPException(
            502, "パッケージを取得できません。通信許可先を確認してください"
        )
    return {"ok": True}


@app.post("/{action}")
async def snapshot(action: str, body: dict):
    from snapshots import capture, restore, validate_snapshot

    if action not in ("snapshot", "restore-snapshot", "validate-snapshot"):
        raise HTTPException(404)
    if RUN_TASKS:
        raise HTTPException(409, "MCP operations are still running")
    for _, task in CONNECTIONS.values():
        task.cancel()
    await asyncio.gather(
        *(task for _, task in CONNECTIONS.values()), return_exceptions=True
    )
    CONNECTIONS.clear()
    if action == "snapshot":
        return capture(SHARED)
    if action == "validate-snapshot":
        validate_snapshot(body)
        return {"ok": True}
    return restore(SHARED, body)
