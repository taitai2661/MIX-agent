#!/usr/bin/env python3
"""Fail on relative links in documentation that point at nothing.

`docs/README.md` links with bare filenames (`architecture.md`), while the top
level READMEs use `docs/...`; both are correct relative to their own file, which
is exactly why a moved file is easy to miss. External URLs and pure anchors are
ignored.

Usage:
    python3 scripts/check-doc-links.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
FILES = [
    ROOT / "README.md",
    ROOT / "README.en.md",
    ROOT / "CONTRIBUTING.md",
    *sorted((ROOT / "docs").rglob("*.md")),
]


def targets(text: str) -> list[str]:
    found = []
    for raw in LINK.findall(text):
        target = raw.split("#", 1)[0].strip().strip("<>")
        if not target or target.startswith(("http://", "https://", "mailto:", "tel:")):
            continue
        found.append(target)
    return found


def main() -> int:
    broken = []
    for file in FILES:
        if not file.exists():
            continue
        for target in targets(unquote(file.read_text(encoding="utf-8"))):
            resolved = (file.parent / target).resolve()
            if not resolved.exists():
                broken.append(f"{file.relative_to(ROOT)}: {target}")
    for entry in broken:
        print(entry, file=sys.stderr)
    if broken:
        print(f"\n{len(broken)} broken relative link(s).", file=sys.stderr)
        return 1
    print(f"Documentation links OK ({len(FILES)} files).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
