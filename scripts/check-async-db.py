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
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
API_DIR = ROOT / "apps" / "server" / "mix_agent" / "api"

# Request reads that do not make a handler's DB work asynchronous.
BODY_READS = {
    "request.json",
    "request.body",
    "request.form",
    "request.files",
    "request.stream",
}


def route_handlers(tree: ast.AST):
    for node in ast.walk(tree):
        if not isinstance(node, (ast.AsyncFunctionDef, ast.FunctionDef)):
            continue
        decorators = [ast.unparse(item) for item in node.decorator_list]
        if any("router." in item for item in decorators):
            yield node


def await_targets(node: ast.AsyncFunctionDef) -> set[str]:
    targets: set[str] = set()
    for item in ast.walk(node):
        if isinstance(item, ast.Await):
            targets.add(ast.unparse(item.value))
    return targets


def main() -> int:
    failures: list[str] = []
    for path in sorted(API_DIR.glob("routes_*.py")):
        source = path.read_text()
        tree = ast.parse(source, filename=str(path))
        for node in route_handlers(tree):
            if not isinstance(node, ast.AsyncFunctionDef):
                continue
            targets = await_targets(node)
            if not targets:
                failures.append(
                    f"{path.relative_to(ROOT)}:{node.lineno} {node.name}: "
                    "async route has no await; declare it with `def` so sync DB work "
                    "runs in FastAPI's threadpool"
                )
            elif targets <= BODY_READS:
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

    print(f"async route / sync DB check passed ({len(list(API_DIR.glob('routes_*.py')))} modules)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
