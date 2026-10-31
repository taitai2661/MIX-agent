#!/usr/bin/env python3
"""Keep README.md and README.en.md structurally identical.

The two files are translated by hand, so prose is expected to differ while the
structure is not: the same heading levels (so the table of contents and anchor
links stay aligned), the same commands (a command documented in one language
but missing in the other is a real gap) and the same link targets.

Run with no arguments. Exit code 1 prints the concrete drift.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

HEADING = re.compile(r"^(#{1,6})\s+(.*)$", re.MULTILINE)
FENCE = re.compile(r"^```[^\n]*\n(.*?)^```", re.MULTILINE | re.DOTALL)
LINK = re.compile(r"\[[^\]]*\]\(([^)#]+)(?:#[^)]*)?\)")
COMMAND = re.compile(r"^\s*(?:pnpm|cargo|docker|git|node|python3?|uv|npm|rustup)\b.*$", re.MULTILINE)


def headings(text: str) -> list[int]:
    return [len(match.group(1)) for match in HEADING.finditer(text)]


def commands(text: str) -> set[str]:
    found: set[str] = set()
    for block in FENCE.findall(text):
        for line in COMMAND.findall(block):
            # Drop the trailing `# comment` so each language can annotate the
            # same command in its own words.
            normalized = " ".join(line.split(" #", 1)[0].split())
            if normalized and not normalized.startswith(("#", "//")):
                found.add(normalized)
    return found


def links(text: str) -> set[str]:
    ignored = {"README.md", "README.en.md"}
    return {
        target.strip()
        for target in LINK.findall(text)
        if not target.startswith("http") and target.strip() not in ignored
    }


def compare(name: str, left: set[str] | list[int], right: set[str] | list[int]) -> bool:
    if left == right:
        return True
    only_left = sorted(set(left) - set(right))
    only_right = sorted(set(right) - set(left))
    print(f"{name}: README.md and README.en.md differ", file=sys.stderr)
    for item in only_left:
        print(f"  only in README.md: {item}", file=sys.stderr)
    for item in only_right:
        print(f"  only in README.en.md: {item}", file=sys.stderr)
    return False


def main() -> int:
    ja = (ROOT / "README.md").read_text(encoding="utf-8")
    en = (ROOT / "README.en.md").read_text(encoding="utf-8")
    ok = True
    ok &= compare("heading levels", headings(ja), headings(en))
    ok &= compare("commands", commands(ja), commands(en))
    ok &= compare("relative links", links(ja), links(en))
    if not ok:
        print("\nREADME parity failed: update both languages together.", file=sys.stderr)
        return 1
    print("README parity OK.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
