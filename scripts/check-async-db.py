#!/usr/bin/env python3
"""Reject routes declared ``async`` that only perform synchronous work.

The server runs one uvicorn worker and uses synchronous SQLAlchemy sessions
(see ``api/routes_runs.py`` and ``main.py``).  An ``async def`` endpoint that
does not genuinely await anything therefore blocks every other request for the
duration of its database work.  Such handlers belong in FastAPI's threadpool,
which means a plain ``def``.

Two shapes are rejected:

* ``async def`` with no ``await`` at all;
* ``async def`` whose only awaits are request-body reads.  Re-reading the JSON
  body does not make the SQLAlchemy work asynchronous, so the handler should
  take a Pydantic body parameter and be synchronous.

Genuine awaits (provider HTTP calls, runner requests, embeddings, …) are
allowed: those handlers have to stay asynchronous, and their remaining DB
work is deliberately short.
"""

from __future__ import annotations

import ast
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
API_DIR = ROOT / "apps" / "server" / "mix_agent" / "api"
MAIN_FILE = ROOT / "apps" / "server" / "mix_agent" / "main.py"

# The server uses Python 3.12 syntax (`match` statements), so `ast.parse` on
# an older interpreter dies with "invalid syntax" and no hint about the real
# cause. CI pins 3.12; point a developer at the project's own interpreter.
if sys.version_info < (3, 12):
    sys.exit(
        "check-async-db.py needs Python 3.12+ to parse the server "
        f"(got {sys.version.split()[0]}). Try: "
        "apps/server/.venv/bin/python scripts/check-async-db.py"
    )

# Request reads that do not make a handler's DB work asynchronous.
BODY_READS = {
    "request.json",
    "request.body",
    "request.form",
    "request.files",
    "request.stream",
}

# FastAPI routes are declared both on APIRouters (`@router.get(...)`) and on
# the application itself (`@app.get(...)` in main.py). A word boundary keeps
# names like `myapp.get` from being mistaken for the application object.
APP_ROUTE = re.compile(r"\bapp\.(?:get|post|put|patch|delete|head|options)\s*\(")
# Static asset handlers in main.py touch no DB/session; they may stay async.
NO_DB_ROUTES = {"push_service_worker", "frontend"}


def route_handlers(tree: ast.AST):
    for node in ast.walk(tree):
        if not isinstance(node, (ast.AsyncFunctionDef, ast.FunctionDef)):
            continue
        decorators = [ast.unparse(item) for item in node.decorator_list]
        if any("router." in item or APP_ROUTE.search(item) for item in decorators):
            yield node


def own_nodes(node: ast.AST):
    """Descendants of `node`, excluding nested function/lambda bodies."""
    for child in ast.iter_child_nodes(node):
        if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)):
            continue
        yield child
        yield from own_nodes(child)


def await_targets(node: ast.AsyncFunctionDef) -> set[str]:
    """Awaits executed by `node` itself, not by nested callbacks.

    `ast.walk` descends into nested `async def`/`lambda` bodies. A handler that
    only awaits `request.json()` but defines a nested coroutine with a genuine
    provider call therefore used to pass the gate: the nested await never runs
    on the request path. Stop at nested function scopes while still walking
    control-flow nodes (loops, `if`, `try`, comprehensions, …) of the handler.
    """
    targets: set[str] = set()
    for child in own_nodes(node):
        if isinstance(child, ast.Await):
            value = child.value
            # Unparse the callee, not the whole call: `await request.json()`
            # has to compare equal to the BODY_READS entry `request.json`.
            targets.add(ast.unparse(value.func if isinstance(value, ast.Call) else value))
    return targets


def streams_response(node: ast.AsyncFunctionDef) -> bool:
    """SSE routes hand their awaits to the returned response iterator.

    `routes_runs.run_events` returns a `StreamingResponse` whose async generator
    does the polling. The route itself runs only a short session lookup, but the
    response is only correct with an async handler, so nested generator awaits
    must not be read as fake justification — nor as a blocking violation.
    """
    return any(
        isinstance(child, ast.Return)
        and child.value is not None
        and "StreamingResponse" in ast.unparse(child.value)
        for child in own_nodes(node)
    )


def api_modules() -> list[pathlib.Path]:
    """Every module under `api/`, not just the `routes_*.py` files.

    The check used to glob `routes_*.py` only, so `api/backups.py`,
    `api/message_context.py`, `api/provider_sync.py` and `api/helpers/*` were
    never inspected: a blocking async route added there passed the gate
    silently. `api/backups.py` in particular holds the request-scoped session
    across a multi-minute backup call.
    """
    modules = [
        path
        for path in API_DIR.rglob("*.py")
        if path.name != "__init__.py" and "__pycache__" not in path.parts
    ]
    # Routes on the app object itself (not a router) live in main.py. Include
    # it so @app.* handlers are covered by the same gate.
    if MAIN_FILE.exists():
        modules.append(MAIN_FILE)
    return sorted(modules)


def main() -> int:
    failures: list[str] = []
    modules = api_modules()
    for path in modules:
        source = path.read_text()
        tree = ast.parse(source, filename=str(path))
        for node in route_handlers(tree):
            if not isinstance(node, ast.AsyncFunctionDef):
                continue
            if node.name in NO_DB_ROUTES:
                continue
            targets = await_targets(node)
            streaming = streams_response(node)
            if not targets and not streaming:
                failures.append(
                    f"{path.relative_to(ROOT)}:{node.lineno} {node.name}: "
                    "async route has no await; declare it with `def` so sync DB work "
                    "runs in FastAPI's threadpool"
                )
            elif targets and targets <= BODY_READS and not streaming:
                failures.append(
                    f"{path.relative_to(ROOT)}:{node.lineno} {node.name}: "
                    "only awaits request-body reads; take a Pydantic body parameter "
                    "and declare the route with `def`"
                )

    if failures:
        print("async route / sync DB check failed:\n", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(f"async route / sync DB check passed ({len(modules)} modules)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
