"""Authenticated, single-flight Playwright Chromium installer."""
import asyncio
import json
import os
import secrets
from pathlib import Path

from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse

app = FastAPI(docs_url=None, redoc_url=None)
ROOT = Path(os.getenv("PLAYWRIGHT_BROWSERS_PATH", "/browser"))
STATE = ROOT / "install-state.json"
LOCK = asyncio.Lock()

def current():
    try:
        return json.loads(STATE.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return {"status": "not_installed", "failure": None}

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

async def install():
    async with LOCK:
        if current().get("status") == "ready":
            return
        save({"status": "installing", "failure": None})
        process = await asyncio.create_subprocess_exec("playwright", "install", "--only-shell", "chromium")
        code = await process.wait()
        save({"status": "ready", "failure": None} if code == 0 else {"status": "failed", "failure": "Playwright Chromiumの導入に失敗しました。"})

@app.post("/install")
async def start_install():
    if LOCK.locked():
        return {"status": "installing", "failure": None}
    if current().get("status") != "ready":
        asyncio.create_task(install())
    return current() if current().get("status") == "ready" else {"status": "installing", "failure": None}
