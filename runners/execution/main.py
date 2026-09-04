"""Fixed single-user runner. No provider keys, database, host mounts, or Docker socket."""

import asyncio
import base64
import io
import os
import signal
import secrets
from pathlib import Path, PurePosixPath
from uuid import uuid4
from fastapi import FastAPI, Request, HTTPException
from pydantic import BaseModel, Field
from playwright.async_api import async_playwright
from pypdf import PdfReader

app = FastAPI(docs_url=None, redoc_url=None)
ROOT = Path(os.getenv("WORKSPACE", "/workspace"))
PROCESSES = {}
BROWSER = None
PLAYWRIGHT = None
CONTEXT = None
PAGE = None
BROWSER_CONFIG = None
LOCK = asyncio.Lock()


@app.middleware("http")
async def auth(request: Request, call_next):
    expected = (
        "Bearer "
        + Path(os.getenv("TOKEN_FILE", "/tokens/execution.token")).read_text().strip()
    )
    if not secrets.compare_digest(request.headers.get("authorization", ""), expected):
        return __import__("fastapi").responses.JSONResponse(
            {"detail": "Unauthorized"}, 401
        )
    return await call_next(request)


def parts(path):
    p = PurePosixPath(path)
    if ".." in p.parts or (p.is_absolute() and p.parts[:2] != ("/", "workspace")):
        raise ValueError("Path outside workspace")
    values = p.parts[2:] if p.is_absolute() else p.parts
    if any(x in ("..", "") for x in values):
        raise ValueError("Invalid path")
    return values


def parent_fd(path, create=False):
    values = parts(path)
    if not values:
        raise ValueError("A filename is required")
    fd = os.open(ROOT, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for name in values[:-1]:
            if create:
                try:
                    os.mkdir(name, dir_fd=fd)
                except FileExistsError:
                    pass
            new = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = new
        return fd, values[-1]
    except BaseException:
        os.close(fd)
        raise


def read(path):
    fd, name = parent_fd(path)
    try:
        handle = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
        import stat

        if not stat.S_ISREG(os.fstat(handle).st_mode):
            os.close(handle)
            raise ValueError("Only regular files are readable")
        with os.fdopen(handle, "rb") as file:
            content = file.read(2_000_001)
            if len(content) > 2_000_000:
                raise ValueError("File exceeds 2MB; use an explicit bounded terminal operation")
            return content.decode("utf-8", errors="replace")
    finally:
        os.close(fd)


def write(path, content):
    fd, name = parent_fd(path, create=True)
    try:
        # Replace the directory entry atomically; never follow symlinks or hardlink targets.
        temporary = ".mix-" + str(uuid4())
        handle = os.open(
            temporary,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
            0o600,
            dir_fd=fd,
        )
        with os.fdopen(handle, "w") as file:
            file.write(content)
        os.rename(temporary, name, src_dir_fd=fd, dst_dir_fd=fd)
    finally:
        os.close(fd)


async def drain(proc, entry):
    while block := await proc.stdout.read(4096):
        entry["output"] = (entry["output"] + block.decode(errors="replace"))[-64000:]
    entry["exit_code"] = await proc.wait()


def stop(entry):
    proc = entry["process"]
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def browser_config(values):
    values = values or {}
    return {
        "timeout": max(3000, min(60000, int(values.get("browser_timeout_ms", 15000)))),
        "locale": str(values.get("browser_locale", "ja-JP"))[:35] or "ja-JP",
        "user_agent": str(values.get("browser_user_agent", ""))[:500],
        "viewport": {"width": max(320, min(2560, int(values.get("browser_viewport_width", 1280)))),
                     "height": max(320, min(2560, int(values.get("browser_viewport_height", 720))))},
        "block_images": bool(values.get("browser_block_images", False)),
    }


async def page(values=None):
    global PLAYWRIGHT, BROWSER, CONTEXT, PAGE, BROWSER_CONFIG
    config = browser_config(values)
    if BROWSER is None:
        try:
            PLAYWRIGHT = await async_playwright().start()
            BROWSER = await PLAYWRIGHT.chromium.launch(
                headless=True,
                proxy={"server": os.getenv("HTTPS_PROXY", "http://egress-proxy:3128")},
                args=["--disable-dev-shm-usage", "--proxy-bypass-list=<-loopback>"],
            )
        except Exception as exc:
            if PLAYWRIGHT is not None:
                await PLAYWRIGHT.stop()
            PLAYWRIGHT = None
            BROWSER = None
            raise ValueError(
                f"Chromiumを起動できません。Browserが導入されているか確認してください。"
            ) from exc
    if PAGE is None or BROWSER_CONFIG != config:
        if CONTEXT is not None:
            await CONTEXT.close()
        context_args = {
            "accept_downloads": False, "service_workers": "block", "locale": config["locale"],
            "viewport": config["viewport"],
        }
        if config["user_agent"]:
            context_args["user_agent"] = config["user_agent"]
        CONTEXT = await BROWSER.new_context(**context_args)

        async def guard(route):
            from urllib.parse import urlsplit

            if urlsplit(route.request.url).scheme not in ("http", "https"):
                await route.abort()
            else:
                await route.continue_()

        async def block_images(route):
            if config["block_images"] and route.request.resource_type == "image":
                await route.abort()
            else:
                await guard(route)

        await CONTEXT.route("**/*", block_images)
        PAGE = await CONTEXT.new_page()
        PAGE.set_default_timeout(config["timeout"])
        BROWSER_CONFIG = config
    return PAGE


class Execute(BaseModel):
    name: str
    arguments: dict
    run_id: str
    browser_settings: dict = Field(default_factory=dict)


@app.post("/execute")
async def execute(body: Execute):
    name, args = body.name, body.arguments
    try:
        async with LOCK:
            if name == "read_file":
                return {"text": read(args["path"])}
            if name in ("write_file", "edit_file"):
                content = args.get("content", "")
                if name == "edit_file":
                    content = read(args["path"])
                    if not args["old"] or content.count(args["old"]) != 1:
                        raise ValueError("Old text must match exactly once")
                    content = content.replace(args["old"], args["new"], 1)
                write(args["path"], content)
                return {"ok": True, "path": args["path"]}
            if name == "delete_file":
                fd, leaf = parent_fd(args["path"])
                try:
                    os.unlink(leaf, dir_fd=fd)
                finally:
                    os.close(fd)
                return {"ok": True}
            if name == "files_list":
                values = parts(args.get("path", "/workspace"))
                fd = os.open(ROOT, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
                try:
                    for value in values:
                        new = os.open(
                            value,
                            os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=fd,
                        )
                        os.close(fd)
                        fd = new
                    return {"files": sorted(os.listdir(fd))[:1000]}
                finally:
                    os.close(fd)
            if name == "search_files":
                found = []
                for path in ROOT.rglob("*"):
                    if len(found) >= 100:
                        break
                    try:
                        rel = str(path.relative_to(ROOT))
                        text = read(rel)
                        if args["query"] in text:
                            found.append(
                                {
                                    "path": rel,
                                    "excerpt": text[
                                        max(0, text.index(args["query"]) - 100) :
                                    ][:500],
                                }
                            )
                    except (OSError, ValueError):
                        continue
                return {"matches": found}
            if name == "run_terminal":
                if (
                    sum(p["process"].returncode is None for p in PROCESSES.values())
                    >= 8
                ):
                    raise ValueError("Process limit reached")
                env = {
                    k: v
                    for k, v in os.environ.items()
                    if k
                    in (
                        "PATH",
                        "LANG",
                        "HTTP_PROXY",
                        "HTTPS_PROXY",
                        "http_proxy",
                        "https_proxy",
                    )
                }
                env.update(
                    HOME="/workspace", TMPDIR="/tmp", PIP_DISABLE_PIP_VERSION_CHECK="1"
                )
                proc = await asyncio.create_subprocess_exec(
                    "/bin/sh",
                    "-lc",
                    args["command"],
                    cwd=ROOT,
                    env=env,
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.STDOUT,
                    start_new_session=True,
                )
                key = str(uuid4())
                entry = {
                    "process": proc,
                    "output": "",
                    "exit_code": None,
                    "run_id": body.run_id,
                }
                PROCESSES[key] = entry
                entry["reader"] = asyncio.create_task(drain(proc, entry))
                if not args.get("background"):
                    try:
                        await asyncio.wait_for(
                            asyncio.shield(entry["reader"]), timeout=90
                        )
                    except (TimeoutError, asyncio.CancelledError):
                        stop(entry)
                        await entry["reader"]
                    finally:
                        stop(entry)
                return {
                    "process_id": key,
                    "output": entry["output"],
                    "exit_code": entry["exit_code"],
                }
            if name == "process_list":
                return {
                    "processes": [
                        {"id": k, "output": v["output"], "exit_code": v["exit_code"]}
                        for k, v in PROCESSES.items()
                    ]
                }
            if name == "process_stop":
                stop(PROCESSES[args["process_id"]])
                return {"ok": True}
            if name.startswith("browser_"):
                p = await page(body.browser_settings)
                if name == "browser_open":
                    from urllib.parse import urlsplit

                    if urlsplit(args["url"]).scheme not in ("http", "https"):
                        raise ValueError("Only HTTP(S) pages can be opened")
                    await p.goto(args["url"], wait_until="domcontentloaded")
                elif name == "browser_click":
                    await p.locator(args["selector"]).first.click()
                elif name == "browser_type":
                    await p.locator(args["selector"]).first.fill(args["text"])
                elif name == "browser_extract":
                    limit = max(1, min(100, int(args.get("limit", 20) or 20)))
                    items = await p.locator(args["selector"]).evaluate_all(
                        "elements => elements.slice(0, " + str(limit) + ").map(e => ({text: (e.innerText || '').slice(0, 2000), href: e.href || (e.querySelector('a') || {}).href || ''}))"
                    )
                    return {"url": p.url, "count": len(items), "items": items}
                elif name == "browser_wait":
                    timeout = max(1000, min(30000, int(args.get("timeout_ms", 10000) or 10000)))
                    if args.get("selector"):
                        try:
                            await p.locator(args["selector"]).first.wait_for(timeout=timeout)
                            return {"url": p.url, "ready": True}
                        except Exception:
                            return {"url": p.url, "ready": False}
                    await p.wait_for_timeout(timeout)
                    return {"url": p.url, "ready": True}
                elif name == "browser_screenshot":
                    return {
                        "image_base64": base64.b64encode(await p.screenshot()).decode()
                    }
                elif name != "browser_read":
                    raise ValueError("Unknown browser tool")
                return {
                    "url": p.url,
                    "text": (await p.locator("body").inner_text())[:30000],
                }
        raise ValueError("Unknown tool")
    except Exception as exc:
        raise HTTPException(422, "Runner operation failed: " + type(exc).__name__)


@app.post("/cancel")
async def cancel(body: dict):
    for entry in PROCESSES.values():
        if entry["run_id"] == body["run_id"]:
            stop(entry)
    return {"ok": True}


@app.post("/extract-pdf")
async def extract_pdf(body: dict):
    raw = base64.b64decode(body["content"], validate=True)
    if len(raw) > 20 * 1024 * 1024:
        raise HTTPException(413)
    reader = PdfReader(io.BytesIO(raw))
    return {
        "text": "\n".join((p.extract_text() or "") for p in reader.pages[:100])[:50000]
    }


@app.post("/{action}")
async def snapshot(action: str, body: dict):
    from snapshots import capture, restore, validate_snapshot

    if action not in ("snapshot", "restore-snapshot", "validate-snapshot"):
        raise HTTPException(404)
    async with LOCK:
        if any(e["process"].returncode is None for e in PROCESSES.values()):
            raise HTTPException(409, "Stop background processes before backup")
        if action == "snapshot":
            return capture(ROOT)
        if action == "validate-snapshot":
            validate_snapshot(body)
            return {"ok": True}
        return restore(ROOT, body)
