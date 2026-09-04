import asyncio
import os
from contextlib import asynccontextmanager
from pathlib import Path
from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse, FileResponse
from fastapi.staticfiles import StaticFiles
from mix_agent.api.routes import router
from mix_agent import config
from mix_agent.runs.engine import scheduler, TASKS


@asynccontextmanager
async def lifespan(app):
    from mix_agent.storage.backup import recover_interrupted
    from mix_agent.memory.jobs import scheduler as memory_scheduler

    await recover_interrupted()
    task = asyncio.create_task(scheduler())
    memory_task = asyncio.create_task(memory_scheduler())
    yield
    task.cancel()
    memory_task.cancel()
    # Shutdown leaves running markers for crash-safe recovery; do not report success.
    for pending in list(TASKS.values()):
        pending.cancel()
    await asyncio.gather(task, memory_task, *list(TASKS.values()), return_exceptions=True)


app = FastAPI(title="MIX agent", version="0.1.0", lifespan=lifespan)


@app.middleware("http")
async def security_headers(request: Request, call_next):
    from mix_agent.storage import backup

    if backup.ACTIVE and request.method not in ("GET", "HEAD", "OPTIONS"):
        return JSONResponse({"detail": "バックアップ／復元処理中です"}, status_code=503)
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
            return JSONResponse({"detail": "Origin is not allowed"}, status_code=403)
        if request.headers.get("sec-fetch-site") == "cross-site":
            return JSONResponse({"detail": "Cross-site request blocked"}, status_code=403)
    response = await call_next(request)
    response.headers["X-Content-Type-Options"] = "nosniff"
    response.headers["X-Frame-Options"] = "DENY"
    response.headers["Referrer-Policy"] = "same-origin"
    response.headers["Content-Security-Policy"] = (
        "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'"
    )
    return response


@app.exception_handler(RequestValidationError)
async def validation_error(request, exc):
    return JSONResponse(
        {"detail": [{"loc": e["loc"], "msg": e["msg"]} for e in exc.errors()]}, status_code=422
    )


@app.get("/health")
async def health():
    from mix_agent.db.session import engine
    from sqlalchemy import text

    with engine.connect() as conn:
        conn.execute(text("SELECT 1"))
    return {"status": "ok"}


from mix_agent.api.backups import router as backup_router

app.include_router(backup_router)
app.include_router(router)
web = Path(os.getenv("WEB_DIST", "apps/web/dist"))
if (web / "assets").exists():
    app.mount("/assets", StaticFiles(directory=web / "assets"), name="assets")


@app.get("/{path:path}")
async def frontend(path: str):
    if path.startswith("api/") or not (web / "index.html").exists():
        return JSONResponse({"detail": "Not found"}, status_code=404)
    return FileResponse(web / "index.html")
