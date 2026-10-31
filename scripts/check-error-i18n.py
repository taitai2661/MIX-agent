#!/usr/bin/env python3
"""Keep the Docker API's error messages, codes, and English text in sync.

Run from the repository root (CI does this in the `memory-parity` job):

    python3 scripts/check-error-i18n.py

Three sets must agree:

* every literal ``HTTPException(..., "message")`` in ``apps/server/mix_agent``
  has an entry in ``mix_agent/api/error_codes.py``;
* every explicit ``APIError(status, "code")`` names a code in that table;
* every code has an English string in ``apps/web/src/app/errorCodes.ts``;
* there are no orphan codes or translations.

A code may stay in the table after its Japanese message stops being raised if
an ``APIError`` still names it: the table doubles as the canonical
code → message mapping, so the server can keep an older message for logs while
new call sites pass only the code.  This is what lets routes migrate to
``APIError`` incrementally.

Dynamic details (``str(exc)``) are intentionally skipped: the server emits a
status-based ``common.http_<status>`` fallback for those.
"""

from __future__ import annotations

import ast
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SERVER = ROOT / "apps" / "server" / "mix_agent"
CODES_FILE = SERVER / "api" / "error_codes.py"
EN_FILE = ROOT / "apps" / "web" / "src" / "app" / "errorCodes.ts"


def literals() -> tuple[set[str], set[str]]:
    """Return (message literals, explicit ``APIError`` code literals)."""
    messages: set[str] = set()
    explicit_codes: set[str] = set()
    for path in SERVER.rglob("*.py"):
        try:
            tree = ast.parse(path.read_text(encoding="utf-8"))
        except SyntaxError as exc:
            print(f"error: cannot parse {path}: {exc}", file=sys.stderr)
            raise SystemExit(2) from exc
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call):
                continue
            func = node.func
            name = func.id if isinstance(func, ast.Name) else getattr(func, "attr", "")
            if name == "HTTPException":
                args = list(node.args)
                detail = args[1] if len(args) > 1 else None
                for keyword in node.keywords:
                    if keyword.arg == "detail":
                        detail = keyword.value
                if isinstance(detail, ast.Constant) and isinstance(detail.value, str) and detail.value:
                    messages.add(detail.value)
            elif name == "APIError":
                # APIError(status, code, detail=None): only the code literal is
                # part of the machine contract.
                args = list(node.args)
                code = args[1] if len(args) > 1 else None
                for keyword in node.keywords:
                    if keyword.arg == "code":
                        code = keyword.value
                if isinstance(code, ast.Constant) and isinstance(code.value, str) and code.value:
                    explicit_codes.add(code.value)
            elif name == "JSONResponse":
                # Middleware rejects requests before routing and therefore
                # cannot raise HTTPException.  Its literal `detail` strings need
                # the same stable code + English text as routed errors.
                if not node.args or not isinstance(node.args[0], ast.Dict):
                    continue
                for key, value in zip(node.args[0].keys, node.args[0].values):
                    if (
                        isinstance(key, ast.Constant)
                        and key.value == "detail"
                        and isinstance(value, ast.Constant)
                        and isinstance(value.value, str)
                        and value.value
                    ):
                        messages.add(value.value)
    return messages, explicit_codes


def error_codes() -> dict[str, str]:
    tree = ast.parse(CODES_FILE.read_text(encoding="utf-8"))
    for node in tree.body:
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            if node.target.id == "ERROR_CODES":
                return ast.literal_eval(node.value)
    raise SystemExit(f"error: ERROR_CODES not found in {CODES_FILE}")


def english_codes() -> set[str]:
    text = EN_FILE.read_text(encoding="utf-8")
    # The object is plain data; match top-level string keys only.
    return set(re.findall(r'^\s*"([a-z0-9_.]+)"\s*:', text, flags=re.MULTILINE))


def main() -> int:
    messages, explicit_codes = literals()
    codes = error_codes()
    english = english_codes()

    problems: list[str] = []

    missing_codes = sorted(messages - set(codes))
    for message in missing_codes:
        problems.append(f"missing code: {message!r}")

    # A code survives an extracted/renamed message while code-first call sites
    # still name it; only codes used by neither form are orphans.
    orphan_codes = sorted(
        message for message, code in codes.items() if message not in messages and code not in explicit_codes
    )
    for message in orphan_codes:
        problems.append(f"code for a message that is no longer raised: {message!r}")

    unknown_explicit = sorted(explicit_codes - set(codes.values()))
    for code in unknown_explicit:
        problems.append(f"APIError code is not defined in ERROR_CODES: {code!r}")

    duplicates = sorted({code for code in codes.values() if list(codes.values()).count(code) > 1})
    for code in duplicates:
        problems.append(f"duplicate code: {code}")

    missing_en = sorted(set(codes.values()) - english)
    for code in missing_en:
        problems.append(f"missing English text: {code}")

    fallback = {code for code in english if code.startswith("common.http_")}
    orphan_en = sorted(english - set(codes.values()) - fallback)
    for code in orphan_en:
        problems.append(f"orphan English text: {code}")

    if problems:
        print(f"{len(problems)} error-i18n problem(s):", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1

    print(f"error-i18n parity OK: {len(messages)} messages, {len(codes)} codes, {len(english)} translations")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
