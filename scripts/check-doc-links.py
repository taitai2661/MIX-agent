#!/usr/bin/env python3
"""Fail on relative links in documentation that point at nothing.

`docs/README.md` links with bare filenames (`architecture.md`), while the top
level READMEs use `docs/...`; both are correct relative to their own file, which
is exactly why a moved file is easy to miss. External URLs and pure `mailto:`
links are ignored; `#anchor` fragments are checked against the headings of the
target Markdown file.

Usage:
    python3 scripts/check-doc-links.py
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
LINK_START = re.compile(r"\[[^\]\n]*\]\(")
HEADING = re.compile(r"^\s{0,3}#{1,6}\s+(.*?)\s*$")
FENCE = re.compile(r"^\s{0,3}(`{3,}|~{3,})")


def tracked_markdown() -> list[Path]:
    """All Markdown files, including new ones not yet staged (local runs)."""
    args = [
        "git",
        "-C",
        str(ROOT),
        "ls-files",
        "-z",
        "--",
        "*.md",
    ]
    output = subprocess.run(args, check=True, capture_output=True).stdout
    names = [name for name in output.decode("utf-8", "replace").split("\0") if name]
    untracked = subprocess.run(
        [
            "git",
            "-C",
            str(ROOT),
            "ls-files",
            "-z",
            "--others",
            "--exclude-standard",
            "--",
            "*.md",
        ],
        check=True,
        capture_output=True,
    ).stdout
    names += [name for name in untracked.decode("utf-8", "replace").split("\0") if name]
    return sorted({ROOT / name for name in names})


def anchors(text: str) -> set[str]:
    """GitHub-style heading anchors (lowercased, punctuation stripped)."""
    result: set[str] = set()
    used: dict[str, int] = {}
    for line in text.splitlines():
        match = HEADING.match(line)
        if not match:
            continue
        heading = re.sub(r"\s+#+\s*$", "", match.group(1))
        heading = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", heading)
        heading = re.sub(r"<[^>]+>", "", heading)
        heading = re.sub(r"[`*_~]", "", heading)
        anchor = re.sub(r"[^\w\s-]", "", heading.strip().lower(), flags=re.UNICODE)
        anchor = re.sub(r"\s+", "-", anchor)
        if not anchor:
            continue
        if anchor in used:
            used[anchor] += 1
            anchor = f"{anchor}-{used[anchor]}"
        else:
            used[anchor] = 0
        result.add(anchor)
    return result


def without_fenced_code(text: str) -> str:
    """Blank out fenced code blocks so example links are not checked."""
    lines: list[str] = []
    fence: str | None = None
    for line in text.splitlines(keepends=True):
        match = FENCE.match(line)
        if match:
            marker = match.group(1)
            if fence is None:
                fence = marker[0] * len(marker)
            elif marker[0] == fence[0] and len(marker) >= len(fence):
                fence = None
            lines.append("\n" if line.endswith("\n") else "")
            continue
        lines.append(
            "\n" if fence is not None and line.endswith("\n") else ("" if fence is not None else line)
        )
    return "".join(lines)


def link_targets(text: str):
    """Yield markdown link destinations, allowing balanced parentheses.

    A regex like ``[^)]+`` stops at the first ``)`` inside a URL such as
    ``https://example.org/foo_(bar)``, reporting a broken relative link.
    """
    for match in LINK_START.finditer(text):
        start = match.end()
        depth = 1
        index = start
        while index < len(text) and depth:
            char = text[index]
            if char == "\\":
                index += 2
                continue
            if char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            if depth:
                index += 1
        if depth == 0:
            yield text[start:index]


def targets(text: str) -> list[tuple[str, str]]:
    found = []
    for raw in link_targets(without_fenced_code(text)):
        raw = unquote(raw)
        target, _, fragment = raw.partition("#")
        target = target.strip().strip("<>")
        fragment = fragment.strip()
        if not target and not fragment:
            continue
        if target.startswith(("http://", "https://", "mailto:", "tel:")):
            continue
        found.append((target, fragment))
    return found


def main() -> int:
    files = tracked_markdown()
    anchor_cache: dict[Path, set[str]] = {}
    broken = []
    for file in files:
        if not file.is_file():
            continue
        text = file.read_text(encoding="utf-8")
        for target, fragment in targets(text):
            resolved = file.resolve() if not target else (file.parent / target).resolve()
            label = f"{file.relative_to(ROOT)}: {target or '.'}"
            if not resolved.exists():
                broken.append(label)
                continue
            if fragment and resolved.suffix.lower() in (".md", ".markdown"):
                if resolved not in anchor_cache:
                    anchor_cache[resolved] = anchors(resolved.read_text(encoding="utf-8"))
                if fragment not in anchor_cache[resolved]:
                    broken.append(f"{label}#{fragment} (heading not found)")
    for entry in broken:
        print(entry, file=sys.stderr)
    if broken:
        print(f"\n{len(broken)} broken relative link(s).", file=sys.stderr)
        return 1
    print(f"Documentation links OK ({len(files)} files).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
