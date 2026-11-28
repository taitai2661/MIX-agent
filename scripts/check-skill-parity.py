#!/usr/bin/env python3
"""Cross-edition contract check for the skill package limits.

Both editions accept the same ``SKILL.md`` packages and enforce the same caps,
but the constants live in separate files. A change to one side (for example,
raising the per-file limit) would let an import succeed in Docker and fail in
Desktop, or store a package one edition refuses to export.

Checked here:

  * ``FILE_MAX`` / ``FILES_TOTAL_MAX`` / ``FILES_COUNT_MAX``
  * the ignored resource names (``node_modules``, ``.git``, …)

Usage::

    python3 scripts/check-skill-parity.py
"""

from __future__ import annotations

import ast
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "apps/desktop/core/src/skills/discovery.rs"
PYTHON = ROOT / "apps/server/mix_agent/skills/package.py"

INT_CONSTS = ("FILE_MAX", "FILES_TOTAL_MAX", "FILES_COUNT_MAX")
RUST_INT = re.compile(r"pub\s+const\s+([A-Z0-9_]+)\s*:\s*usize\s*=\s*([^;]+);")
RUST_NAMES = re.compile(r"pub\s+const\s+IGNORED_NAMES\s*:\s*\[&str;\s*\d+\]\s*=\s*\[(.*?)\];", re.S)
PY_NAMES = re.compile(r"^IGNORED_NAMES\s*=\s*frozenset\(\s*\{(.*?)\}\s*\)", re.S | re.M)


def fail(message: str) -> None:
    print(f"skill-parity: {message}", file=sys.stderr)
    raise SystemExit(1)


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        fail(f"cannot read {path.relative_to(ROOT)}: {exc}")


def arithmetic_int(expression: str) -> int:
    """Integer arithmetic from repo sources, without ``eval``.

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


def evaluate(value: str, source: str) -> int:
    expression = value.replace("_", "").strip()
    try:
        return arithmetic_int(expression)
    except Exception:  # noqa: BLE001 - an unparseable constant is itself drift
        fail(f"cannot evaluate {source} value {value!r}")


def strings(block: str) -> set[str]:
    return set(re.findall(r'"([^"]+)"', block)) | set(re.findall(r"'([^']+)'", block))


def main() -> None:
    rust_text = read(RUST)
    python_text = read(PYTHON)

    rust_ints = {name: evaluate(value, f"Rust {name}") for name, value in RUST_INT.findall(rust_text)}
    python_ints: dict[str, int] = {}
    for node in ast.parse(python_text).body:
        if (
            isinstance(node, ast.Assign)
            and len(node.targets) == 1
            and isinstance(node.targets[0], ast.Name)
            and node.targets[0].id in INT_CONSTS
        ):
            try:
                python_ints[node.targets[0].id] = arithmetic_int(ast.unparse(node.value))
            except (ValueError, SyntaxError, TypeError):
                fail(f"Python {node.targets[0].id} is not an integer expression")

    for name in INT_CONSTS:
        if name not in rust_ints:
            fail(f"missing Rust constant {name}")
        if name not in python_ints:
            fail(f"missing Python constant {name}")
        if rust_ints[name] != python_ints[name]:
            fail(f"{name} drift: Rust={rust_ints[name]} Python={python_ints[name]}")

    rust_match = RUST_NAMES.search(rust_text)
    python_match = PY_NAMES.search(python_text)
    if not rust_match:
        fail("missing Rust IGNORED_NAMES")
    if not python_match:
        fail("missing Python IGNORED_NAMES")
    rust_ignored = strings(rust_match.group(1))
    python_ignored = strings(python_match.group(1))
    if rust_ignored != python_ignored:
        missing = sorted(rust_ignored - python_ignored)
        extra = sorted(python_ignored - rust_ignored)
        fail(f"IGNORED_NAMES drift: Rust-only={missing} Python-only={extra}")

    print(
        "skill-parity: ok "
        f"({', '.join(f'{name}={rust_ints[name]}' for name in INT_CONSTS)}; "
        f"{len(rust_ignored)} ignored names)"
    )


if __name__ == "__main__":
    main()
