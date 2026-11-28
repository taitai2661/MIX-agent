#!/usr/bin/env python3
"""Fail when a credential-shaped string is committed.

This is a deliberately small, dependency-free substitute for gitleaks: it scans
all tracked files with a handful of high-signal patterns (provider key
prefixes, private-key headers, JWTs) instead of payload entropy. That keeps it
deterministic and fast enough to run on every pull request, and it cannot be
disabled by a corrupted allowlist file.

CI scans every tracked text file under the size limit by default: a credential
under tests/ or docs/ is still a real credential. Binary and oversized files are
skipped and reported as a count on stderr. For a fast local iteration loop, opt
out explicitly with ``--skip-fixtures`` (CI never does).

Usage:
    python3 scripts/check-secrets.py
    python3 scripts/check-secrets.py --skip-fixtures   # local speed only

``--all`` is accepted as a deprecated no-op so older commands keep working.

Exit code 1 lists every finding as `path:line: rule`.
"""

from __future__ import annotations

import hashlib
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
        # Named group so the allowlist keys on the credential *value*, not on
        # the surrounding `access_token="..."` formatting.
        re.compile(
            r"""(?i)(?:password|passwd|secret|api[_-]?key|access[_-]?token)\s*[:=]\s*["'](?P<value>[^"'${}<>\s]{12,})["']"""
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

# Versioned allowlist for the deliberate fixture values that still look like
# credentials. Entries are keyed on (path, rule, sha256(captured value)) — never
# on a whole file, line range, or the surrounding assignment syntax — so:
#   * reformatting a fixture (`access_token = 'x'` vs `access_token="x"`) does
#     not break the allowlist, and
#   * a newly added *different* credential in one of these files still fails.
# Bump ALLOWLIST_VERSION when reviewing an entry and keep the list as small as
# possible; every item needs a concrete fixture reason.
ALLOWLIST_VERSION = 2
# Kept as fragments so this script's own source does not match the rules below
# when the full scan reads it.
_GITHUB_FIXTURE = "ghp_" + "abcdefghijklmnopqrstuvwxyz" + "0123456789"
_ALLOWLIST_ENTRIES = (
    (
        "apps/desktop/tools/src/redact.rs",
        "github-token",
        _GITHUB_FIXTURE,
        "redactor tests assert that a real-shaped GitHub token is masked",
    ),
    (
        "tests/integration/test_providers.py",
        "hardcoded-credential",
        "access-token",
        "synthetic ChatGPT credential fixture, never used against a provider",
    ),
    (
        "tests/unit/test_chatgpt.py",
        "hardcoded-credential",
        "final-access",
        "synthetic ChatGPT refresh fixture, never used against a provider",
    ),
)


def value_fingerprint(value: str) -> str:
    """Stable, formatting-independent key for a deliberately allowed value."""
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


ALLOWLIST = {
    (path, rule, value_fingerprint(value)): reason for path, rule, value, reason in _ALLOWLIST_ENTRIES
}

# Paths whose *content* is intentionally credential-shaped: tests build fake
# tokens, docs show examples, and this file contains the patterns. The full
# scan still reads these files; --skip-fixtures is an explicit local-only opt
# out. The allowlist above covers the handful of matches that are deliberate.
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


def is_placeholder(value: str) -> bool:
    """Exact-token match, never a substring match.

    Substring matching let a real credential through whenever its value merely
    contained the word "dummy" or "placeholder" (e.g. a key leaked into a test
    fixture). Only a value that *is* a known placeholder is ignored.
    """
    return value.strip().casefold() in PLACEHOLDERS


def scan(path: str, text: str) -> list[tuple[str, int, str, str]]:
    findings = []
    for number, line in enumerate(text.splitlines(), start=1):
        # Long lines used to be skipped wholesale; minified assets and lockfiles
        # are tracked files too, so scan every line regardless of its length.
        for rule, pattern in RULES:
            for match in pattern.finditer(line):
                value = match.groupdict().get("value") or match.group(0)
                if is_placeholder(value):
                    continue
                findings.append((path, number, rule, value))
    return findings


def main() -> int:
    args = sys.argv[1:]
    skip_fixtures = "--skip-fixtures" in args
    if "--all" in args:
        print(
            "note: --all is deprecated (the full scan is now the default); ignoring it.",
            file=sys.stderr,
        )
    if skip_fixtures:
        print(
            "note: --skip-fixtures omits credential-shaped content under "
            + ", ".join(SKIP_PREFIXES)
            + "; CI always runs the full scan.",
            file=sys.stderr,
        )
    findings = []
    allowed = 0
    unscanned: list[str] = []
    for path in tracked_files():
        if skip_fixtures and skipped(path):
            continue
        file = ROOT / path
        try:
            if file.stat().st_size > MAX_BYTES:
                unscanned.append(f"{path} (> {MAX_BYTES} bytes)")
                continue
            data = file.read_bytes()
        except OSError:
            continue
        if b"\0" in data:
            unscanned.append(f"{path} (binary)")
            continue
        for found_path, number, rule, value in scan(path, data.decode("utf-8", "replace")):
            if (found_path, rule, value_fingerprint(value)) in ALLOWLIST:
                allowed += 1
                continue
            findings.append(f"{found_path}:{number}: {rule}")
    if allowed:
        print(
            f"note: {allowed} deliberate fixture match(es) covered by the "
            f"versioned allowlist (v{ALLOWLIST_VERSION}).",
            file=sys.stderr,
        )
    if unscanned:
        print(
            f"note: {len(unscanned)} binary/oversized file(s) were not scanned "
            "(text content under the size limit only).",
            file=sys.stderr,
        )
    for finding in findings:
        print(finding)
    if findings:
        print(f"\n{len(findings)} potential secret(s) found.", file=sys.stderr)
        return 1
    print("No credential-shaped strings found.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
