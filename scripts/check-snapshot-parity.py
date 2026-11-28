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

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DOCKERFILE = ROOT / "deploy/docker/Dockerfile"
CANONICAL = "packages/tool_contracts/snapshots.py"
REQUIRED_COPY = f"COPY {CANONICAL} /app/snapshots.py"
# One copy for the `execution` stage and one for the `mcp` stage.
EXPECTED_COPIES = 2
COPY_LINE = re.compile(r"^COPY\s+")


def code_part(line: str) -> str:
    """Strip a trailing Dockerfile comment so commented-out copies do not count."""
    return re.sub(r"\s+", " ", line.split("#", 1)[0]).strip()


def main() -> int:
    text = DOCKERFILE.read_text(encoding="utf-8")
    copies = [
        code
        for line in text.splitlines()
        if (code := code_part(line)) and COPY_LINE.match(code) and "snapshots.py" in code
    ]
    if not copies:
        print(f"FAIL: Dockerfile has no COPY line for snapshots.py (expected {EXPECTED_COPIES})")
        return 1
    bad = [line for line in copies if line != REQUIRED_COPY]
    if bad:
        for line in bad:
            print(f"FAIL: non-canonical snapshot copy: {line}")
        return 1
    if len(copies) != EXPECTED_COPIES:
        print(
            f"FAIL: expected {EXPECTED_COPIES} COPY lines ({REQUIRED_COPY}), "
            f"found {len(copies)}: one of the execution/mcp stages is missing or duplicated"
        )
        return 1
    if not (ROOT / CANONICAL).is_file():
        print(f"FAIL: missing canonical helper: {CANONICAL}")
        return 1
    print(f"snapshot parity OK: {len(copies)} COPY lines use {CANONICAL}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
