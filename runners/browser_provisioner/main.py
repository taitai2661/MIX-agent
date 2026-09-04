"""Authenticated, single-flight Playwright Chromium installer."""
import asyncio
import json
import logging
import os
import secrets
from pathlib import Path

from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse

app = FastAPI(docs_url=None, redoc_url=None)
ROOT = Path(os.getenv("PLAYWRIGHT_BROWSERS_PATH", "/browser"))
STATE = ROOT / "install-state.json"
LOCK = asyncio.Lock()
LOGGER = logging.getLogger(__name__)

def current():
    try:
        value = json.loads(STATE.read_text())
        if isinstance(value, dict) and value.get("status") in {
            "not_installed", "installing", "ready", "failed"
        }:
            return value
        return {"status": "failed", "failure": "install_state_invalid"}
    except (FileNotFoundError, json.JSONDecodeError):
        return {"status": "failed", "failure": "install_state_unavailable"}

def save(value):
    ROOT.mkdir(parents=True, exist_ok=True)
    temp = STATE.with_suffix(".tmp")
    temp.write_text(json.dumps(value))
    temp.replace(STATE)

@app.middleware("http")
async def authenticate(request: Request, call_next):
    token = Path(os.getenv("TOKEN_FILE", "/tokens/browser-provisioner.token")).read_text().strip()
    if not secrets.compare_digest(request.headers.get("authorization", ""), "Bearer " + token):
        return JSONResponse({"detail": "Unauthorized"}, 401)
    return await call_next(request)

@app.post("/status")
async def status():
    return current()

INSTALL_TIMEOUT = 600  # 10 minutes


async def install():
    async with LOCK:
        if current().get("status") == "ready":
            return
        save({"status": "installing", "failure": None})
        process = None
        try:
            process = await asyncio.create_subprocess_exec(
                "playwright", "install", "--only-shell", "chromium",
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT,
            )
            stdout, _ = await asyncio.wait_for(process.communicate(), timeout=INSTALL_TIMEOUT)
            if process.returncode == 0:
                save({"status": "ready", "failure": None})
            else:
                LOGGER.error("Playwright Chromium installation failed with exit code %s: %s",
                             process.returncode,
                             stdout.decode(errors="replace")[-2000:] if stdout else "")
                save({"status": "failed", "failure": "install_failed"})
        except asyncio.TimeoutError:
            if process is not None:
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
                await process.communicate()
            save({"status": "failed", "failure": "install_timeout"})
        except Exception:
            LOGGER.exception("Playwright Chromium installation failed")
            save({"status": "failed", "failure": "install_error"})

@app.post("/install")
async def start_install():
    state = current()
    if state.get("status") == "ready":
        return state
    if LOCK.locked():
        return {"status": "installing", "failure": None}
    asyncio.create_task(install())
    return {"status": "installing", "failure": None}
