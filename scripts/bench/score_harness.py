#!/usr/bin/env python3
"""Score public coding-harness runs (template).

Mirrors the scoring rules of ``apps/desktop/core/tests/coding_eval.rs`` so a
result published under ``docs/benchmarks/`` means the same thing as the CI
harness: the agent's prose is never evidence. A task passes only when the
project's own check (``verify``) succeeds in a separate process **and** the
expected change is present in the files. ``false_claim`` is recorded when the
final answer says the work is done while the independent check still fails.

Two commands::

    # 1. Sanity-check the fixtures (no agent, no model):
    #    installs each fixture into a temp dir and asserts the check FAILS
    #    there — a fixture that already passes cannot measure anything.
    python3 scripts/bench/score_harness.py --check-fixtures

    # 2. Score a finished run into the published format:
    python3 scripts/bench/score_harness.py --score run-results.json \\
        --out docs/benchmarks/2026-11-09-harness.json \\
        --run-mode live --model qwen2.5-coder-32b-instruct

``run-results.json`` is a JSON object mapping each fixture task name to its
raw outcome::

    {
      "single_file_bugfix": {
        "steps": 4,
        "tool_calls": 3,
        "prompt_tokens": 5200,
        "wall_ms": 18200,
        "status": "completed",
        "final_text": "Fixed `add` in `calc.py`; tests pass.",
        "event_kinds": ["tool_executed", "verification_passed"],
        "workspace": "/tmp/agent-workspaces/single_file_bugfix"
      },
      ...
    }

``workspace`` is the directory holding that task's final files; the scorer
runs ``verify`` there with ``sh -c`` (independent process, like the Rust
harness). Guardrail tasks additionally need ``event_kinds`` (the engine event
names observed during the run). Fields besides ``workspace`` may be omitted;
they are recorded as ``null`` and excluded from the budget check.

The published report carries ``ok`` and ``violations`` so a failed or
budget-regressed run can never look like a clean measurement after the fact.

Exit status: 0 when every scored task passes AND the scripted budgets hold
(<= 12 steps per task, < 200 000 prompt tokens across the three coding
tasks); 1 when any task fails or a budget regresses; 2 on usage errors.
"""

from __future__ import annotations

import argparse
import json
import platform
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURES_PATH = Path(__file__).resolve().parent / "harness_fixtures.json"

# Scripted budgets from coding_eval.rs (regression net, not model quality).
MAX_STEPS_PER_TASK = 12
MAX_PROMPT_TOKENS_CODING_TASKS = 200_000
VERIFY_TIMEOUT_SECS = 120

EXIT_OK = 0
EXIT_REGRESSION = 1
EXIT_ERROR = 2

DEFINITIONS = {
    "result": "independent verify_command in a separate process AND every expected substring present (guardrail tasks: see pass_condition)",
    "steps": "provider turns consumed for the task (driver of prompt-cost growth)",
    "tools": "tool calls executed during the run",
    "prompt_tok": "estimated prompt tokens for the run (Rust harness: requests chars / 4)",
    "wall_ms": "wall-clock milliseconds; machine-dependent, never used for pass/fail",
    "claim": "final answer claims success while the independent check still fails",
}

# Same success vocabulary as claims_success() in coding_eval.rs.
CLAIM_NEEDLES = (
    "tests pass",
    "test passes",
    "all tests",
    "fixed",
    "done",
    "completed",
    "成功",
    "完了",
    "通りました",
)


def progress(message: str) -> None:
    print(f"score: {message}", file=sys.stderr, flush=True)


def load_fixtures() -> dict:
    try:
        return json.loads(FIXTURES_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise SystemExit(f"cannot load fixtures {FIXTURES_PATH}: {exc}") from exc


def claims_success(text: str) -> bool:
    lowered = (text or "").lower()
    return any(needle in lowered for needle in CLAIM_NEEDLES)


def run_verify(verify: str | None, workspace: Path) -> tuple[bool, str]:
    """Run the project's own check outside the agent. Returns (ok, output)."""
    if verify is None:
        return True, ""
    try:
        completed = subprocess.run(
            ["sh", "-c", verify],
            cwd=workspace,
            capture_output=True,
            text=True,
            timeout=VERIFY_TIMEOUT_SECS,
        )
    except subprocess.TimeoutExpired:
        return False, f"verify timed out after {VERIFY_TIMEOUT_SECS}s: {verify}"
    combined = (completed.stdout or "") + (completed.stderr or "")
    return completed.returncode == 0, combined.strip()


def expected_present(task: dict, workspace: Path) -> tuple[bool, str]:
    """Check every expected substring is in the final files."""
    for item in task.get("expected", []):
        target = workspace / item["path"]
        try:
            content = target.read_text(encoding="utf-8")
        except OSError:
            return False, f"missing file {item['path']}"
        if item["needle"] not in content:
            return False, f"{item['path']} lacks {item['needle']!r}"
    return True, "all expected substrings present" if task.get("expected") else "no file expectations"


def install_fixture(task: dict, workspace: Path) -> None:
    for item in task["files"]:
        target = workspace / item["path"]
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(item["content"], encoding="utf-8")


def score_task(task: dict, raw: dict) -> dict:
    """Score one task. ``raw`` holds the agent run's metrics + workspace."""
    workspace = Path(raw["workspace"]) if raw.get("workspace") else None
    if workspace is None or not workspace.is_dir():
        return {
            "task": task["name"],
            "passed": False,
            "steps": raw.get("steps"),
            "tool_calls": raw.get("tool_calls"),
            "prompt_tokens": raw.get("prompt_tokens"),
            "wall_ms": raw.get("wall_ms"),
            "status": raw.get("status"),
            "false_claim": False,
            "detail": f"workspace missing or not a directory: {raw.get('workspace')}",
        }
    verified, output = run_verify(task.get("verify"), workspace)
    expected_ok, expected_note = expected_present(task, workspace)
    kinds = list(raw.get("event_kinds", []))
    guardrail = task.get("guardrail", "none")

    if guardrail == "none":
        passed = verified and expected_ok
        detail = (
            "task completed and verified"
            if passed
            else f"verified={verified} expected={expected_ok} ({expected_note}): {output}"
        )
    elif guardrail == "verification_gate":
        gated = "verification_failed" in kinds
        passed = gated and not verified
        detail = (
            "engine rejected the unverified conclusion"
            if gated
            else "engine let an unverified conclusion through"
        )
    elif guardrail == "plan_read_only":
        gated = "verification_required" in kinds or "verification_failed" in kinds
        passed = expected_ok and not gated
        if not expected_ok:
            detail = f"plan mode wrote to the workspace ({expected_note})"
        elif gated:
            detail = "a refused edit still armed the verification gate"
        else:
            detail = "plan mode left the workspace untouched"
    else:
        passed, detail = False, f"unknown guardrail {guardrail!r}"

    return {
        "task": task["name"],
        "passed": passed,
        "steps": raw.get("steps"),
        "tool_calls": raw.get("tool_calls"),
        "prompt_tokens": raw.get("prompt_tokens"),
        "wall_ms": raw.get("wall_ms"),
        "status": raw.get("status"),
        "false_claim": claims_success(raw.get("final_text", "")) and not verified,
        "detail": detail,
    }


def check_fixtures(fixtures: dict) -> int:
    """Assert every coding fixture FAILS its check when freshly installed."""
    failures = 0
    for task in fixtures["tasks"]:
        with tempfile.TemporaryDirectory(prefix="bench-fixture-") as tmp:
            workspace = Path(tmp)
            install_fixture(task, workspace)
            verified, output = run_verify(task.get("verify"), workspace)
            if task.get("guardrail", "none") == "none" and task.get("verify"):
                if verified:
                    print(f"FAIL {task['name']}: check already passes on a fresh fixture")
                    failures += 1
                else:
                    progress(f"{task['name']}: fresh fixture fails as intended")
            else:
                progress(f"{task['name']}: guardrail fixture (verify exit={verified})")
            _ = output
    if failures:
        print(f"{failures} fixture(s) cannot measure anything", file=sys.stderr)
        return EXIT_REGRESSION
    print("fixtures ok: coding tasks fail before the fix, guardrails carry pass_condition")
    return EXIT_OK


def build_report(fixtures: dict, args: argparse.Namespace, scores: list[dict]) -> dict:
    # Classify by the fixture's declared guardrail, never by a hard-coded task
    # name: renaming a fixture used to silently move it into the coding bucket
    # and let it pass through the wrong scoring path.
    guardrail_by_name = {task["name"]: task.get("guardrail", "none") for task in fixtures["tasks"]}

    def is_guardrail(score: dict) -> bool:
        return guardrail_by_name.get(score["task"], "none") != "none"

    coding = [s for s in scores if not is_guardrail(s)]
    guards = [s for s in scores if is_guardrail(s)]
    steps_vals = [s["steps"] for s in scores if isinstance(s.get("steps"), (int, float))]
    prompt_total = sum(s["prompt_tokens"] for s in scores if isinstance(s.get("prompt_tokens"), (int, float)))
    coding_prompt = sum(
        s["prompt_tokens"] for s in coding if isinstance(s.get("prompt_tokens"), (int, float))
    )
    return {
        "tool": "apps/desktop/core/tests/coding_eval.rs",
        "spec": "docs/desktop/coding-eval.md",
        "generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "harness_version": fixtures.get("source", "") + " + scripts/bench/score_harness.py",
        "environment": {
            "os": platform.system(),
            "arch": platform.machine(),
            "python": platform.python_version(),
            "rust_harness": "see Cargo.toml (run `cargo test -p mixd-core --test coding_eval`)",
        },
        "run": {
            "mode": args.run_mode,
            "model": args.model,
            "base_url": args.base_url,
            "timeout_secs": args.timeout_secs,
            "context_window": args.context_window,
        },
        "thresholds": {
            "max_steps_per_task": MAX_STEPS_PER_TASK,
            "max_prompt_tokens_coding_tasks": MAX_PROMPT_TOKENS_CODING_TASKS,
        },
        "tasks": scores,
        "summary": {
            "tasks_passed": f"{sum(1 for s in coding if s['passed'])}/{len(coding)}",
            "guardrails_passed": f"{sum(1 for s in guards if s['passed'])}/{len(guards)}",
            "avg_steps": round(sum(steps_vals) / len(steps_vals), 1) if steps_vals else None,
            "total_prompt_tokens": prompt_total if scores else None,
            "coding_prompt_tokens": coding_prompt if coding else None,
        },
        "definitions": DEFINITIONS,
    }


def find_violations(report: dict, scores: list[dict]) -> list[str]:
    """Everything that makes this run unsuitable to publish as a baseline."""
    violations: list[str] = []
    for score in scores:
        steps = score.get("steps")
        if isinstance(steps, (int, float)) and steps > MAX_STEPS_PER_TASK:
            violations.append(f"{score['task']}: {steps} steps exceed {MAX_STEPS_PER_TASK}")
    coding_prompt = report["summary"]["coding_prompt_tokens"]
    if isinstance(coding_prompt, (int, float)) and coding_prompt >= MAX_PROMPT_TOKENS_CODING_TASKS:
        violations.append(f"prompt cost regressed: {coding_prompt} tokens across the coding tasks")
    violations.extend(f"{s['task']} failed" for s in scores if not s["passed"])
    return violations


def format_report(report: dict) -> str:
    lines = ["coding harness — public bench", f"mode={report['run']['mode']} model={report['run']['model']}"]
    lines.append(f"{'task':<36} {'result':>6} {'steps':>5} {'tools':>6} {'prompt_tok':>10} {'claim':>6}")
    for score in report["tasks"]:

        def cell(value: object) -> str:
            return "n/a" if value is None else str(value)

        lines.append(
            f"{score['task']:<36} "
            f"{('PASS' if score['passed'] else 'FAIL'):>6} "
            f"{cell(score.get('steps')):>5} "
            f"{cell(score.get('tool_calls')):>6} "
            f"{cell(score.get('prompt_tokens')):>10} "
            f"{('FALSE' if score.get('false_claim') else '-'):>6}"
        )
        if not score["passed"]:
            lines.append(f"  {score['detail']}")
    summary = report["summary"]
    lines.append(
        f"summary: tasks {summary['tasks_passed']}, guardrails {summary['guardrails_passed']}, "
        f"avg steps {summary['avg_steps']}, prompt tokens {summary['total_prompt_tokens']}"
    )
    lines.append(f"ok={report['ok']} violations={report['violations']}")
    return "\n".join(lines)


def cmd_score(args: argparse.Namespace) -> int:
    fixtures = load_fixtures()
    try:
        raw_results = json.loads(Path(args.score).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"bench error: cannot load results {args.score}: {exc}", file=sys.stderr)
        return EXIT_ERROR
    if isinstance(raw_results, list):
        raw_results = {item.pop("task"): item for item in raw_results}
    if not isinstance(raw_results, dict):
        print("bench error: results must be an object keyed by task name", file=sys.stderr)
        return EXIT_ERROR

    scores = []
    for task in fixtures["tasks"]:
        raw = raw_results.get(task["name"])
        if raw is None:
            scores.append(
                {
                    "task": task["name"],
                    "passed": False,
                    "steps": None,
                    "tool_calls": None,
                    "prompt_tokens": None,
                    "wall_ms": None,
                    "status": None,
                    "false_claim": False,
                    "detail": "task not present in results file",
                }
            )
        else:
            scores.append(score_task(task, raw))

    report = build_report(fixtures, args, scores)
    # Budgets only apply where the run actually measured the metric.
    violations = find_violations(report, scores)
    report["ok"] = not violations
    report["violations"] = violations
    print(format_report(report))
    for line in violations:
        print(f"bench: REGRESSION {line}", file=sys.stderr, flush=True)

    # Decide before writing: the published JSON always carries the verdict and
    # the full violation list, so a failed run cannot be mistaken for a baseline.
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    progress(f"report written to {out}")
    return EXIT_REGRESSION if violations else EXIT_OK


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Score public coding-harness runs the way coding_eval.rs does (independent verifier only).",
    )
    parser.add_argument(
        "--check-fixtures", action="store_true", help="validate the fixture set (no agent needed)"
    )
    parser.add_argument("--score", metavar="RESULTS_JSON", help="raw per-task outcomes to score")
    parser.add_argument(
        "--out", metavar="PATH", help="write the docs/benchmarks/<date>-harness.json report here"
    )
    parser.add_argument(
        "--run-mode", choices=("scripted", "live"), default="live", help="how the run was produced"
    )
    parser.add_argument("--model", default=None, help="model id on the wire (live mode)")
    parser.add_argument("--base-url", default=None, help="OpenAI-compatible base URL (live mode)")
    parser.add_argument("--timeout-secs", type=int, default=180, help="per-task limit used for the run")
    parser.add_argument("--context-window", type=int, default=0, help="context length; 0 = unknown")
    args = parser.parse_args(argv)
    if args.check_fixtures and args.score:
        parser.error("--check-fixtures and --score are mutually exclusive")
    if bool(args.score) != bool(args.out):
        parser.error("--score and --out must be given together")
    if not args.check_fixtures and not args.score:
        parser.error("one of --check-fixtures or --score is required")
    return args


def main(argv: list[str] | None = None) -> int:
    try:
        args = parse_args(argv)
    except SystemExit as exc:
        return EXIT_ERROR if exc.code != 0 else EXIT_OK
    if args.check_fixtures:
        try:
            return check_fixtures(load_fixtures())
        except SystemExit as exc:
            print(f"bench error: {exc}", file=sys.stderr)
            return EXIT_ERROR
    return cmd_score(args)


if __name__ == "__main__":
    sys.exit(main())
