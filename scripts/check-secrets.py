#!/usr/bin/env python3
"""Fail when a credential-shaped string is committed.

This is a deliberately small, dependency-free substitute for gitleaks: it scans
tracked files with a handful of high-signal patterns (provider key prefixes,
private-key headers, JWTs) instead of payload entropy. That keeps it
deterministic and fast enough to run on every pull request, and it cannot be
disabled by a corrupted allowlist file.

Usage:
    python3 scripts/check-secrets.py
    python3 scripts/check-secrets.py --all      # include test fixtures/docs

Exit code 1 lists every finding as `path:line: rule`.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

RULES = (
    ("aws-access-key", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    ("github-token", re.compile(r"\bgh[pousr]_[A-Za-z0-9]{36,}\b")),
    ("openai-key", re.compile(r"\bsk-(?:proj-)?[A-Za-z0-9_-]{32,}\b")),
    ("anthropic-key", re.compile(r"\bsk-ant-[A-Za-z0-9_-]{20,}\b")),
    ("slack-token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b")),
    ("google-api-key", re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b")),
    ("private-key", re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH |PGP )?PRIVATE KEY-----")),
    ("jwt", re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b")),
    (
        "hardcoded-credential",
        re.compile(
            r"""(?i)(?:password|passwd|secret|api[_-]?key|access[_-]?token)\s*[:=]\s*["'][^"'${}<>\s]{12,}["']"""
        ),
    ),
)

# Placeholder values that exist on purpose in fixtures and examples. Compared
# case-insensitively against the captured value.
PLACEHOLDERS = {
    "test-only-password",
    "preview-only-password",
    "test-password-12345",
    "test-password",
    "private-value",
    "changeme",
    "example-password",
    "your-password",
    "your-api-key",
    "placeholder",
    "dummy",
}

# Paths whose *content* is intentionally credential-shaped: tests build
# fake tokens, docs show examples, and this file contains the patterns.
SKIP_PREFIXES = (
    "tests/",
    "docs/",
    "experiments/",
    "scripts/check-secrets.py",
    "apps/desktop/ui/src/lib/i18n/",
    # Unit tests for the redactor assert on real token shapes they mask.
    "apps/desktop/tools/src/redact.rs",
)

MAX_BYTES = 2_000_000


def tracked_files() -> list[str]:
    output = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z"],
        check=True,
        capture_output=True,
    ).stdout
    return [name for name in output.decode("utf-8", "replace").split("\0") if name]


def skipped(path: str) -> bool:
    return any(path.startswith(prefix) for prefix in SKIP_PREFIXES)


def scan(path: str, text: str) -> list[str]:
    findings = []
    for number, line in enumerate(text.splitlines(), start=1):
        if len(line) > 4096:
            continue
        for rule, pattern in RULES:
            for match in pattern.finditer(line):
                value = match.group(0).lower()
                if any(placeholder in value for placeholder in PLACEHOLDERS):
                    continue
                findings.append(f"{path}:{number}: {rule}")
    return findings


def main() -> int:
    include_skipped = "--all" in sys.argv[1:]
    findings = []
    for path in tracked_files():
        if not include_skipped and skipped(path):
            continue
        file = ROOT / path
        try:
            if file.stat().st_size > MAX_BYTES:
                continue
            data = file.read_bytes()
        except OSError:
            continue
        if b"\0" in data:
            continue
        findings.extend(scan(path, data.decode("utf-8", "replace")))
    for finding in findings:
        print(finding)
    if findings:
        print(f"\n{len(findings)} potential secret(s) found.", file=sys.stderr)
        return 1
    print("No credential-shaped strings found.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
