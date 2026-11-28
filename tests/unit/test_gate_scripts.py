"""Regression tests for the repository's CI gate scripts.

These scripts are normally only exercised through CI, so a subtle narrowing of
what they inspect can silently turn the gate green. Keep the few behaviours
that caused false passes here as cheap unit tests.
"""

from __future__ import annotations

import ast
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load_script(name: str):
    path = ROOT / "scripts" / f"{name}.py"
    spec = importlib.util.spec_from_file_location(f"_{name}", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


check_async_db = load_script("check-async-db")


def test_route_handlers_recognize_app_and_router_decorators():
    source = """\
from fastapi import FastAPI
app = FastAPI()

@app.get("/main")
async def on_app():
    await request.json()

@router.post("/api")
async def on_router():
    await request.json()

def undecorated():
    pass
"""
    handlers = [node.name for node in check_async_db.route_handlers(ast.parse(source))]
    assert handlers == ["on_app", "on_router"]


def test_await_targets_ignores_nested_function_and_lambda_bodies():
    source = """\
async def run_events(request):
    await request.json()
    async def nested():
        await provider_call()
    callback = lambda: request.body()
    return nested, callback
"""
    function = ast.parse(source).body[0]
    targets = check_async_db.await_targets(function)
    assert targets == {"request.json"}
    assert targets <= check_async_db.BODY_READS


def test_await_targets_keeps_awaits_in_control_flow():
    source = """\
async def handler():
    if True:
        await provider_call()
    for item in items:
        await other_call()
"""
    function = ast.parse(source).body[0]
    assert check_async_db.await_targets(function) == {"provider_call", "other_call"}
