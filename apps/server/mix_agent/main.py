import asyncio
import logging
import os
import secrets
import time
import uuid
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.openapi.utils import get_openapi
from fastapi.responses import FileResponse, JSONResponse, PlainTextResponse
from fastapi.staticfiles import StaticFiles
from starlette.exceptions import HTTPException as StarletteHTTPException

from mix_agent import config, metrics
from mix_agent.api.error_codes import APIError, code_for_detail
from mix_agent.logging_setup import configure_logging
from mix_agent.api.routes import router
from mix_agent.api.routes_artifacts import router as artifacts_router
from mix_agent.api.routes_auth import router as auth_router
from mix_agent.api.routes_conversations import router as conversations_router
from mix_agent.api.routes_mcp import router as mcp_router
from mix_agent.api.routes_memories import router as memories_router
from mix_agent.api.routes_messages import router as messages_router
from mix_agent.api.routes_notifications import router as notifications_router
from mix_agent.api.routes_providers import router as providers_router
from mix_agent.api.routes_runs import router as runs_router
from mix_agent.api.routes_schedules import router as schedules_router
from mix_agent.api.routes_settings import router as settings_router
from mix_agent.api.routes_skills import router as skills_router
from mix_agent.api.routes_statistics import router as statistics_router
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


# Import-time, after uvicorn has installed its handlers: only swaps the
# formatter when MIX_LOG_FORMAT=json is set.
configure_logging()

app = FastAPI(title="MIX agent", version="0.5.4", lifespan=lifespan)
LOGGER = logging.getLogger("mix_agent.http")


def _openapi_with_errors():
    """Publish the ``{detail, code}`` error envelope in the schema.

    The handlers already return two fields, but without a component clients
    (and the generated TypeScript API types) saw only ``detail``. Every
    operation gets the shared 4XX/5XX response so the contract is visible at
    the call site instead of being tribal knowledge.
    """
    if app.openapi_schema:
        return app.openapi_schema
    schema = get_openapi(title=app.title, version=app.version, routes=app.routes)
    components = schema.setdefault("components", {}).setdefault("schemas", {})
    components["ErrorResponse"] = {
        "type": "object",
        "required": ["detail"],
        "properties": {
            "detail": {
                "description": (
                    "Canonical Japanese message, or the field list for a 422 validation error; "
                    "older clients and logs still read it."
                ),
                "oneOf": [
                    {"type": "string"},
                    {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "loc": {"type": "array", "items": {}},
                                "msg": {"type": "string"},
                            },
                        },
                    },
                ],
            },
            "code": {
                "type": "string",
                "description": "Stable machine-readable code (see `mix_agent/api/error_codes.py`).",
            },
        },
    }
    envelope = {
        "description": "Error envelope; `code` stays stable when the message wording changes.",
        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ErrorResponse"}}},
    }
    for operations in schema.get("paths", {}).values():
        for operation in operations.values():
            if not isinstance(operation, dict) or "responses" not in operation:
                continue
            operation["responses"].setdefault("4XX", envelope)
            operation["responses"].setdefault("5XX", envelope)
    app.openapi_schema = schema
    return schema


app.openapi = _openapi_with_errors


@app.middleware("http")
async def security_headers(request: Request, call_next):
    from mix_agent.storage import backup

    request_id = request.headers.get("x-request-id") or uuid.uuid4().hex
    started = time.perf_counter()
    # Early rejects never reach the router, so they are counted as one bounded
    # bucket.  They still carry a stable code so the English UI can translate
    # them (`scripts/check-error-i18n.py` scans these literals too).
    early = True
    try:
        if backup.ACTIVE and request.method not in ("GET", "HEAD", "OPTIONS"):
            response = JSONResponse(
                {"detail": "バックアップ／復元処理中です", "code": "common.backup_in_progress"},
                status_code=503,
            )
        elif request.method not in ("GET", "HEAD", "OPTIONS"):
            origin = request.headers.get("origin")
            if origin and not await asyncio.to_thread(
                config.allowed_origin,
                origin,
                scheme=request.url.scheme,
                host=request.headers.get("host", ""),
                forwarded=any(
                    request.headers.get(header)
                    for header in ("forwarded", "x-forwarded-host", "x-forwarded-proto")
                ),
            ):
                response = JSONResponse(
                    {"detail": "許可されていないOriginです", "code": "common.origin_not_allowed"},
                    status_code=403,
                )
            elif request.headers.get("sec-fetch-site") == "cross-site":
                response = JSONResponse(
                    {"detail": "クロスサイトリクエストは拒否されました", "code": "common.cross_site_blocked"},
                    status_code=403,
                )
            else:
                early = False
                response = await call_next(request)
        else:
            early = False
            response = await call_next(request)
    except Exception:
        # ServerErrorMiddleware catches this after us; count the 500 here so
        # unhandled exceptions are visible in `/metrics` at all.
        if request.url.path != "/metrics":
            label = "/middleware" if early else metrics.route_label(request.scope)
            metrics.observe(request.method, label, 500, time.perf_counter() - started)
        raise
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
    if request.url.path != "/metrics":
        label = "/middleware" if early else metrics.route_label(request.scope)
        metrics.observe(request.method, label, response.status_code, time.perf_counter() - started)
    return response


@app.exception_handler(RequestValidationError)
async def validation_error(request, exc):
    return JSONResponse(
        {"detail": [{"loc": e["loc"], "msg": e["msg"]} for e in exc.errors()]}, status_code=422
    )


@app.exception_handler(StarletteHTTPException)
async def http_error(request, exc: StarletteHTTPException):
    """Attach a stable machine code to every HTTP error.

    ``detail`` stays as the Japanese message for logs and older clients; the
    code lets the UI translate without matching on the sentence.  Dynamic
    details still get a status-based code so users never see raw Japanese in an
    English UI.
    """
    detail = exc.detail
    body = {"detail": detail}
    # An explicit `APIError(code=...)` wins; the message table is the fallback
    # for the older call sites.
    code = getattr(exc, "code", None) or code_for_detail(detail)
    if code is None:
        # Even a bare `HTTPException(404)` gets a translatable code; a missing
        # detail must not leave the English UI without one.
        code = f"common.http_{exc.status_code}"
    if code:
        body["code"] = code
    return JSONResponse(body, status_code=exc.status_code, headers=exc.headers)


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


@app.get("/metrics", response_class=PlainTextResponse)
def prometheus_metrics(request: Request):
    """Scrape the in-process counters.

    Set ``METRICS_TOKEN`` to require ``Authorization: Bearer <token>``.  It is
    off by default for single-user self-hosting, but a deployment that exposes
    the app beyond loopback should set it (or restrict the path in the proxy).
    """
    token = os.getenv("METRICS_TOKEN", "")
    if token:
        supplied = request.headers.get("authorization", "").removeprefix("Bearer ").strip()
        if not secrets.compare_digest(supplied, token):
            raise APIError(401, "auth.login_required")
    return PlainTextResponse(metrics.render(), media_type="text/plain; version=0.0.4")


from mix_agent.api.backups import router as backup_router

app.include_router(backup_router)
app.include_router(auth_router)
# Explicit provider/artifact routes must be registered before the generic
# `/{collection}/{key}` routes in `router` so they keep precedence.
app.include_router(providers_router)
app.include_router(artifacts_router)
app.include_router(notifications_router)
app.include_router(memories_router)
app.include_router(settings_router)
app.include_router(statistics_router)
app.include_router(schedules_router)
# Conversations owns the explicit `/projects/{key}` route, so it must be
# registered before the generic `/{collection}/{key}` router below.
app.include_router(messages_router)
app.include_router(conversations_router)
app.include_router(runs_router)
app.include_router(skills_router)
app.include_router(mcp_router)
app.include_router(router)
web = Path(os.getenv("WEB_DIST", "apps/web/dist"))
if (web / "assets").exists():
    app.mount("/assets", StaticFiles(directory=web / "assets"), name="assets")


@app.get("/push-sw.js")
async def push_service_worker():
    path = web / "push-sw.js"
    if not path.exists():
        return JSONResponse({"detail": "見つかりません", "code": "common.not_found"}, status_code=404)
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
        return JSONResponse({"detail": "見つかりません", "code": "common.not_found"}, status_code=404)
    root = web.resolve()
    asset = (root / path).resolve()
    if path and asset.is_file() and root in asset.parents:
        return FileResponse(asset, media_type=STATIC_MEDIA_TYPES.get(asset.suffix.lower()))
    return FileResponse(web / "index.html")
