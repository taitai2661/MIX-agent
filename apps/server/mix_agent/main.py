import asyncio
import logging
import os
import time
import uuid
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import FileResponse, JSONResponse
from fastapi.staticfiles import StaticFiles

from mix_agent import config
from mix_agent.api.routes import router
from mix_agent.runs.engine import TASKS, scheduler


@asynccontextmanager
async def lifespan(app):
    from mix_agent.memory.jobs import scheduler as memory_scheduler
    from mix_agent.storage.backup import recover_interrupted
    from mix_agent.wakeups import memory_jobs, run_events
    from mix_agent.wakeups import scheduler as scheduler_wakeup

    run_events.bind()
    scheduler_wakeup.bind()
    memory_jobs.bind()
    try:
        await recover_interrupted()
    except Exception:
        # Belt-and-suspenders: a startup-recovery failure must never prevent
        # the web app from serving.
        import logging

        logging.getLogger(__name__).exception("startup recovery failed")
    task = asyncio.create_task(scheduler())
    memory_task = asyncio.create_task(memory_scheduler())
    from mix_agent.push import worker as push_worker

    push_task = asyncio.create_task(push_worker())
    yield
    task.cancel()
    memory_task.cancel()
    push_task.cancel()
    # Shutdown leaves running markers for crash-safe recovery; do not report success.
    from mix_agent.memory.jobs import ACTIVE_JOBS

    for pending in list(TASKS.values()):
        pending.cancel()
    for pending in list(ACTIVE_JOBS):
        pending.cancel()
    await asyncio.gather(
        task, memory_task, push_task, *list(TASKS.values()), *list(ACTIVE_JOBS), return_exceptions=True
    )


app = FastAPI(title="MIX agent", version="0.5.4", lifespan=lifespan)
LOGGER = logging.getLogger("mix_agent.http")


@app.middleware("http")
async def security_headers(request: Request, call_next):
    from mix_agent.storage import backup

    request_id = request.headers.get("x-request-id") or uuid.uuid4().hex
    started = time.perf_counter()
    if backup.ACTIVE and request.method not in ("GET", "HEAD", "OPTIONS"):
        return JSONResponse(
            {"detail": "バックアップ／復元処理中です"},
            status_code=503,
            headers={"X-Request-ID": request_id},
        )
    if request.method not in ("GET", "HEAD", "OPTIONS"):
        origin = request.headers.get("origin")
        if origin and not config.allowed_origin(
            origin,
            scheme=request.url.scheme,
            host=request.headers.get("host", ""),
            forwarded=any(
                request.headers.get(header)
                for header in ("forwarded", "x-forwarded-host", "x-forwarded-proto")
            ),
        ):
            return JSONResponse(
                {"detail": "Origin is not allowed"},
                status_code=403,
                headers={"X-Request-ID": request_id},
            )
        if request.headers.get("sec-fetch-site") == "cross-site":
            return JSONResponse(
                {"detail": "Cross-site request blocked"},
                status_code=403,
                headers={"X-Request-ID": request_id},
            )
    response = await call_next(request)
    response.headers["X-Content-Type-Options"] = "nosniff"
    response.headers["X-Frame-Options"] = "DENY"
    response.headers["Referrer-Policy"] = "same-origin"
    response.headers["X-Request-ID"] = request_id
    response.headers["Permissions-Policy"] = (
        "camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=()"
    )
    response.headers["Cross-Origin-Opener-Policy"] = "same-origin-allow-popups"
    response.headers["Cross-Origin-Resource-Policy"] = "same-origin"
    if request.url.path.startswith("/api/"):
        response.headers.setdefault("Cache-Control", "no-store")
    if config.COOKIE_SECURE:
        response.headers["Strict-Transport-Security"] = "max-age=31536000; includeSubDomains"
    response.headers["Content-Security-Policy"] = (
        "default-src 'self'; script-src 'self' 'sha256-i2aw5SfnpQW4OtZJAk0oWu5VOM5eHaP8Ut4joV2YKUk='; "
        "img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'"
    )
    if request.url.path not in ("/health", "/live"):
        LOGGER.info(
            "request method=%s path=%s status=%s duration_ms=%.1f request_id=%s",
            request.method,
            request.url.path,
            response.status_code,
            (time.perf_counter() - started) * 1000,
            request_id,
        )
    return response


@app.exception_handler(RequestValidationError)
async def validation_error(request, exc):
    return JSONResponse(
        {"detail": [{"loc": e["loc"], "msg": e["msg"]} for e in exc.errors()]}, status_code=422
    )


@app.get("/health")
def health():
    # A plain ``def`` endpoint runs in FastAPI's threadpool; the SQLAlchemy
    # engine is synchronous, so an ``async def`` here would block the event loop.
    from sqlalchemy import text

    from mix_agent.db.session import engine

    with engine.connect() as conn:
        conn.execute(text("SELECT 1"))
    return {"status": "ok"}


@app.get("/live")
def live():
    """Liveness probe: the process is up even while a dependency is down."""
    return {"status": "ok"}


from mix_agent.api.backups import router as backup_router

app.include_router(backup_router)
app.include_router(router)
web = Path(os.getenv("WEB_DIST", "apps/web/dist"))
if (web / "assets").exists():
    app.mount("/assets", StaticFiles(directory=web / "assets"), name="assets")


@app.get("/push-sw.js")
async def push_service_worker():
    path = web / "push-sw.js"
    if not path.exists():
        return JSONResponse({"detail": "Not found"}, status_code=404)
    return FileResponse(path, media_type="application/javascript", headers={"Cache-Control": "no-cache"})


STATIC_MEDIA_TYPES = {
    ".svg": "image/svg+xml",
    ".ico": "image/vnd.microsoft.icon",
    ".png": "image/png",
    ".webp": "image/webp",
    ".webmanifest": "application/manifest+json",
}


@app.get("/{path:path}")
async def frontend(path: str):
    if path == "api" or path.startswith("api/") or not (web / "index.html").exists():
        return JSONResponse({"detail": "Not found"}, status_code=404)
    root = web.resolve()
    asset = (root / path).resolve()
    if path and asset.is_file() and root in asset.parents:
        return FileResponse(asset, media_type=STATIC_MEDIA_TYPES.get(asset.suffix.lower()))
    return FileResponse(web / "index.html")
