#!/usr/bin/env python3
"""Cross-edition browser contract check.

The canonical vocabulary lives in ``packages/tool_contracts/browser.json``.
Desktop embeds it at compile time and exposes a single ``browser`` tool through
the native WebView host; Docker maps ``browser_*`` tools onto the same actions. Without this gate, adding
an action to one edition is invisible to the other and the two UIs drift.

This script fails when:

  * the contract is not valid JSON / has no actions
  * the Desktop ``BrowserAction`` enum and the contract's ``desktop.expose``
    disagree
  * the Docker registry advertises a ``browser_*`` tool that is not in
    ``surfaces.docker.tools`` (or vice versa)
  * the Docker runner does not implement an advertised ``browser_*`` tool
  * an action is neither mapped nor explicitly listed as unsupported for an
    edition
  * a mapped/unsupported name is not a contract action
  * the contract's error or limit blocks are empty

Usage::

    python3 scripts/check-browser-parity.py
"""

from __future__ import annotations

import ast
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "packages/tool_contracts/browser.json"
DESKTOP_ENUM = ROOT / "apps/desktop/browser/src/interface.rs"
DOCKER_REGISTRY = ROOT / "apps/server/mix_agent/tools/registry.py"
DOCKER_RUNNER = ROOT / "runners/execution/main.py"

CAMEL = re.compile(r"(?<!^)(?=[A-Z])")


def camel_to_snake(name: str) -> str:
    return CAMEL.sub("_", name).lower()


def fail(message: str) -> None:
    print(f"browser-parity: {message}", file=sys.stderr)
    raise SystemExit(1)


def load_contract() -> dict:
    try:
        contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot read {CONTRACT.relative_to(ROOT)}: {exc}")
    if not isinstance(contract.get("actions"), dict) or not contract["actions"]:
        fail("contract has no actions")
    if not contract.get("errors") or not contract.get("limits"):
        fail("contract must declare errors and limits")
    return contract


def rust_actions() -> set[str]:
    source = DESKTOP_ENUM.read_text(encoding="utf-8")
    match = re.search(r"pub enum BrowserAction \{(.*?)\n\}", source, re.S)
    if not match:
        fail("could not find BrowserAction in interface.rs")
    body = match.group(1)
    names = re.findall(r"(?m)^    ([A-Z][A-Za-z0-9]*)\s*(?:\{|,)", body)
    if not names:
        fail("BrowserAction enum has no variants")
    return {camel_to_snake(name) for name in names}


def docker_tools() -> set[str]:
    source = DOCKER_REGISTRY.read_text(encoding="utf-8")
    return set(re.findall(r'definition\(\s*"(browser_[a-z_]+)"', source))


def docker_runner_tools() -> set[str]:
    source = DOCKER_RUNNER.read_text(encoding="utf-8")
    # `browser_read` is the fall-through branch (`name != "browser_read"`),
    # so both equality and inequality guards have to be recognised.
    return set(re.findall(r'name (?:==|!=) "(browser_[a-z_]+)"', source))


# Contract limit -> the Rust constant that enforces it. Keep this table small:
# it exists to catch a one-sided tightening (e.g. lowering a DOM cap in Rust
# without updating the shared contract), not to mirror every number by hand.
LIMIT_CONSTS = {
    "max_dom_chars": "MAX_DOM_CHARS",
    "max_text_chars": "MAX_TEXT_CHARS",
    "max_snapshot_nodes": "MAX_SNAPSHOT_NODES",
    "max_screenshot_bytes": "MAX_SCREENSHOT_BYTES",
    "max_evaluate_bytes": "MAX_EVALUATE_SCRIPT_BYTES",
    "max_extract_items": "MAX_EXTRACT_ITEMS",
}


def arithmetic_int(expression: str) -> int:
    """Integer arithmetic from an untrusted source, without ``eval``.

    Only integer literals, ``+ - * //`` and unary ``+ -`` are accepted; names,
    calls, attributes and every other Python construct are rejected.
    """
    try:
        node = ast.parse(expression, mode="eval").body
    except (SyntaxError, ValueError) as exc:
        raise ValueError(f"invalid arithmetic expression {expression!r}") from exc

    def visit(item: ast.AST) -> int:
        if isinstance(item, ast.Constant) and type(item.value) is int:
            return item.value
        if isinstance(item, ast.BinOp) and isinstance(item.op, (ast.Add, ast.Sub, ast.Mult, ast.FloorDiv)):
            left, right = visit(item.left), visit(item.right)
            if isinstance(item.op, ast.Add):
                return left + right
            if isinstance(item.op, ast.Sub):
                return left - right
            if isinstance(item.op, ast.Mult):
                return left * right
            return left // right
        if isinstance(item, ast.UnaryOp) and isinstance(item.op, (ast.UAdd, ast.USub)):
            operand = visit(item.operand)
            return operand if isinstance(item.op, ast.UAdd) else -operand
        raise ValueError(f"disallowed expression node: {type(item).__name__}")

    return visit(node)


def check_limits(limits: dict) -> None:
    source = DESKTOP_ENUM.read_text(encoding="utf-8")
    for key, name in LIMIT_CONSTS.items():
        match = re.search(rf"pub const {name}: usize = ([^;]+);", source)
        if not match:
            fail(f"cannot find Rust constant {name} in interface.rs")
        expression = match.group(1).replace("_", "")
        try:
            value = arithmetic_int(expression)
        except Exception:  # noqa: BLE001 - any unparseable constant is a drift failure
            fail(f"cannot evaluate {name} = {expression!r}")
        if limits.get(key) != value:
            fail(f"limit {key} drift: contract={limits.get(key)} Rust={value}")


def main() -> None:
    contract = load_contract()
    actions = set(contract["actions"])
    desktop_surface = contract["surfaces"]["desktop"]
    desktop_expose = set(desktop_surface["expose"])

    if desktop_expose != actions:
        missing = sorted(actions - desktop_expose)
        extra = sorted(desktop_expose - actions)
        fail(f"desktop.expose mismatch (missing={missing}, extra={extra})")

    manual_only = set(desktop_surface.get("manual_only", []))
    if not manual_only <= actions:
        fail(f"manual_only lists unknown actions: {sorted(manual_only - actions)}")

    check_limits(contract["limits"])

    enum_actions = rust_actions()
    if enum_actions != desktop_expose:
        missing = sorted(desktop_expose - enum_actions)
        extra = sorted(enum_actions - desktop_expose)
        fail(f"BrowserAction enum mismatch (missing={missing}, extra={extra})")

    docker_surface = contract["surfaces"]["docker"]
    docker_tools_map = docker_surface["tools"]
    mapped_tools = set(docker_tools_map)
    mapped_actions = set(docker_tools_map.values())
    unsupported = set(docker_surface["unsupported"])
    overlap = mapped_actions & unsupported
    if overlap:
        fail(f"Docker maps and marks unsupported at once: {sorted(overlap)}")
    if mapped_actions | unsupported != actions:
        uncovered = sorted(actions - mapped_actions - unsupported)
        fail(f"actions missing a Docker disposition: {uncovered}")

    registered = docker_tools()
    if registered != mapped_tools:
        missing = sorted(mapped_tools - registered)
        extra = sorted(registered - mapped_tools)
        fail(f"Docker registry mismatch (missing={missing}, extra={extra})")

    runner = docker_runner_tools()
    if runner != mapped_tools:
        missing = sorted(mapped_tools - runner)
        extra = sorted(runner - mapped_tools)
        fail(f"Docker runner mismatch (missing={missing}, extra={extra})")

    unknown = sorted(mapped_actions - actions)
    if unknown:
        fail(f"Docker tools map to unknown actions: {unknown}")

    print(
        "browser-parity: ok "
        f"({len(actions)} actions, Desktop={len(desktop_expose)}, "
        f"Docker={len(mapped_tools)} mapped + {len(unsupported)} unsupported)"
    )


if __name__ == "__main__":
    main()
