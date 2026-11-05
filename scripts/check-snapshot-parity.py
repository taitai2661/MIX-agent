#!/usr/bin/env python3
"""The execution/mcp images copy the shared snapshot helper into /app.

Both ``COPY packages/tool_contracts/snapshots.py /app/snapshots.py`` lines
must keep pointing at the canonical package file: runners import ``snapshots``
(ales top-level ``/app`` module), so a copy from any other path — or an edit
applied to only one stage — would silently fork the backup/restore contract
between the execution and MCP runners.

Usage: ``python3 scripts/check-snapshot-parity.py``
"""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DOCKERFILE = ROOT / "deploy/docker/Dockerfile"
CANONICAL = "packages/tool_contracts/snapshots.py"


def main() -> int:
    text = DOCKERFILE.read_text(encoding="utf-8")
    copies = [line.strip() for line in text.splitlines() if "snapshots.py" in line]
    if not copies:
        print("FAIL: Dockerfile no longer ships snapshots.py")
        return 1
    bad = [line for line in copies if CANONICAL not in line]
    if bad:
        for line in bad:
            print(f"FAIL: non-canonical snapshot copy: {line}")
        return 1
    if not (ROOT / CANONICAL).is_file():
        print(f"FAIL: missing canonical helper: {CANONICAL}")
        return 1
    print(f"snapshot parity OK: {len(copies)} COPY line(s) use {CANONICAL}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
