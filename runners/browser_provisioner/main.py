"""Authenticated, single-flight Playwright Chromium installer."""
import asyncio
import json
import logging
import os
import re
import secrets
from pathlib import Path

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from playwright.async_api import async_playwright

app = FastAPI(docs_url=None, redoc_url=None)
ROOT = Path(os.getenv("PLAYWRIGHT_BROWSERS_PATH", "/browser"))
STATE = ROOT / "install-state.json"
LOCK = asyncio.Lock()
TASK: asyncio.Task | None = None
LOGGER = logging.getLogger(__name__)
INSTALL_TIMEOUT = 600
PROGRESS = re.compile(rb"(\d{1,3})%")


def current():
    try:
        value = json.loads(STATE.read_text())
        if isinstance(value, dict) and value.get("status") in {
            "not_installed", "installing", "ready", "failed"
        }:
            if value["status"] == "installing" and (TASK is None or TASK.done()):
                value = {"status": "failed", "failure": "install_interrupted",
                         "progress": None, "stage": None}
                save(value)
            return value
        return {"status": "failed", "failure": "install_state_invalid", "progress": None, "stage": None}
    except (FileNotFoundError, json.JSONDecodeError):
        return {"status": "not_installed", "failure": None, "progress": None, "stage": None}


def save(value):
    ROOT.mkdir(parents=True, exist_ok=True)
    temp = STATE.with_suffix(".tmp")
    temp.write_text(json.dumps(value))
    temp.replace(STATE)


def update(progress=None, stage=None):
    previous = current()
    old = previous.get("progress") or 0
    save({"status": "installing", "failure": None,
          "progress": max(old, progress) if progress is not None else previous.get("progress"),
          "stage": stage or previous.get("stage")})


@app.middleware("http")
async def authenticate(request: Request, call_next):
    token = Path(os.getenv("TOKEN_FILE", "/tokens/browser-provisioner.token")).read_text().strip()
    if not secrets.compare_digest(request.headers.get("authorization", ""), "Bearer " + token):
        return JSONResponse({"detail": "Unauthorized"}, 401)
    return await call_next(request)


@app.post("/status")
async def status():
    return current()


async def verify_browser():
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(
            headless=True,
            proxy={"server": os.getenv("HTTPS_PROXY", "http://egress-proxy:3128")},
            args=["--disable-dev-shm-usage", "--proxy-bypass-list=<-loopback>"],
        )
        try:
            page = await browser.new_page()
            await page.goto("data:text/html,<title>ready</title>")
            if await page.title() != "ready":
                raise RuntimeError("Chromium smoke test failed")
        finally:
            await browser.close()


async def run_install(was_ready=False):
    async with LOCK:
        process = None
        verifying = was_ready
        save({"status": "installing", "failure": None, "progress": 0,
              "stage": "起動を確認中" if was_ready else "ダウンロードを準備中"})
        try:
            async with asyncio.timeout(INSTALL_TIMEOUT):
                if not was_ready:
                    process = await asyncio.create_subprocess_exec(
                        "playwright", "install", "--only-shell", "chromium",
                        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT,
                    )
                    output = bytearray()
                    trailing = b""
                    while chunk := await process.stdout.read(4096):
                        output.extend(chunk)
                        if len(output) > 8192:
                            del output[:-4096]
                        for match in PROGRESS.finditer(trailing + chunk):
                            update(min(90, int(match.group(1)) * 9 // 10), "ダウンロード中")
                        trailing = chunk[-8:]
                    if await process.wait() != 0:
                        LOGGER.error("Chromium install failed: %s", bytes(output)[-2000:].decode(errors="replace"))
                        raise RuntimeError("install_failed")
                update(95, "起動を確認中")
                verifying = True
                await verify_browser()
                save({"status": "ready", "failure": None, "progress": 100, "stage": "利用可能"})
        except (TimeoutError, asyncio.CancelledError, Exception) as error:
            if process is not None and process.returncode is None:
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
                await process.wait()
            failure = ("install_timeout" if isinstance(error, TimeoutError) else
                       "install_interrupted" if isinstance(error, asyncio.CancelledError) else
                       "verify_failed" if verifying else "install_failed")
            LOGGER.warning("Chromium installation ended: %s", failure)
            save({"status": "failed", "failure": failure, "progress": None, "stage": None})
            if isinstance(error, asyncio.CancelledError):
                raise


@app.post("/install")
async def start_install():
    global TASK
    if TASK is not None and not TASK.done():
        return current()
    was_ready = current().get("status") == "ready"
    state = {"status": "installing", "failure": None, "progress": 0,
             "stage": "起動を確認中" if was_ready else "ダウンロードを準備中"}
    save(state)
    TASK = asyncio.create_task(run_install(was_ready))
    return state
