#!/usr/bin/env python3
"""Type-check gate with a warning budget.

``pyright`` exits non-zero only for *errors*; the project deliberately keeps a
handful of third-party typing mismatches (parsel, google-genai, chromiumoxide)
as warnings.  Running pyright directly in CI therefore let new warnings slip
in unnoticed.  This wrapper fails when:

* pyright reports any error, or
* the warning count grows past the recorded baseline.

When the count drops, lower ``BASELINE_WARNINGS`` in the same commit so the
ratchet only moves one way.  Run it from the repository root (CI does):

    python3 scripts/check-pyright-budget.py
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SERVER = ROOT / "apps" / "server"

# Current count after fixing the `reliability.profile` annotations and the
# nullable row accesses this script was introduced with.
BASELINE_WARNINGS = 45


def _max_warnings(argv: list[str]) -> int:
    if "--max-warnings" in argv:
        index = argv.index("--max-warnings")
        try:
            return int(argv[index + 1])
        except (IndexError, ValueError):
            raise SystemExit("--max-warnings needs an integer") from None
    return BASELINE_WARNINGS


def main(argv: list[str]) -> int:
    budget = _max_warnings(argv)
    proc = subprocess.run(
        [sys.executable, "-m", "pyright", "--outputjson"],
        cwd=SERVER,
        capture_output=True,
        text=True,
        check=False,
    )
    try:
        report = json.loads(proc.stdout)
    except json.JSONDecodeError:
        # pyright could not run at all (missing dependency, bad config).
        sys.stderr.write(proc.stdout)
        sys.stderr.write(proc.stderr)
        print("pyright-budget: could not parse pyright output", file=sys.stderr)
        return 2

    summary = report.get("summary") or {}
    errors = int(summary.get("errorCount", 0))
    warnings = int(summary.get("warningCount", 0))
    diagnostics = report.get("generalDiagnostics") or []

    def show(severity: str) -> None:
        for diagnostic in diagnostics:
            if diagnostic.get("severity") != severity:
                continue
            file = diagnostic.get("file", "?")
            line = (diagnostic.get("range") or {}).get("start", {}).get("line", 0) + 1
            message = diagnostic.get("message", "")
            print(f"  {file}:{line}: {message}", file=sys.stderr)

    if errors:
        print(f"pyright-budget: {errors} error(s); errors are always fatal", file=sys.stderr)
        show("error")
        return 1

    if warnings > budget:
        print(
            f"pyright-budget: {warnings} warning(s) exceeds the budget of {budget}. "
            "Fix the new warnings or justify and raise BASELINE_WARNINGS.",
            file=sys.stderr,
        )
        show("warning")
        return 1

    print(f"pyright-budget OK: {errors} errors, {warnings} warnings (budget {budget})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
