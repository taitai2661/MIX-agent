#!/usr/bin/env python3
"""Fail when the Run event vocabulary drifts between the two editions.

The Docker engine (Python) and the Desktop engine (Rust) cannot share code, but
their UIs and the spec treat ``run_events.kind`` as one contract. This gate
reads the declarations from each tree and compares them with
``packages/tool_contracts/run_events.json``:

* ``shared``    — emitted by both editions with the same spelling.
* ``docker_only`` / ``desktop_only`` — emitted by one edition only, with the
  reason recorded in the contract. Naming a difference here is a deliberate
  decision, not an accepted surprise.

Adding a kind on one side therefore fails the gate until the contract is
updated, which forces the author to check whether the other edition (and its
UI) needs the same event.

Sources:
  Docker:  ALL_KINDS in apps/server/mix_agent/runs/events.py
  Desktop: string literals passed to the engine's ``.emit(...)`` calls, plus
           synthetic ``RunEvent{View} { kind: ... }`` markers (replay).

Usage:
    python3 scripts/check-run-event-parity.py
"""

from __future__ import annotations

import ast
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "packages/tool_contracts/run_events.json"
DOCKER_EVENTS = ROOT / "apps/server/mix_agent/runs/events.py"
DESKTOP_ENGINE = (
    ROOT / "apps/desktop/core/src/runs/engine.rs",
    ROOT / "apps/desktop/core/src/runs/engine",
)
# The Tauri command layer also emits run events (budget resolution, start
# failures), and builds replay markers directly instead of using the bus.
# `app.emit("local_job_event", ...)` is a Tauri event, not a run event: its
# first argument is a literal, so the second-argument pattern ignores it.
DESKTOP_APP = ROOT / "apps/desktop/app/src"
DESKTOP_MARKERS = (
    ROOT / "apps/desktop/app/src/run_events.rs",
    ROOT / "apps/desktop/app/src/commands/runs.rs",
)

# `.emit(run_id, "kind", payload)`: the first argument is a simple
# identifier/field access, so the literal is the second argument.
EMIT_CALL = re.compile(r'\.emit\(\s*[^,()]+,\s*"([a-z0-9_]+)"')
EMIT_POINT = re.compile(r"\.emit\(")
# Only a call whose first argument is the run id is a run event in the command
# layer; this guard catches a future `emit(run_id, kind, ...)`.
RUN_ID_EMIT = re.compile(r"\.emit\(\s*&?run_id\s*,")
RUN_ID_EMIT_CALL = re.compile(r'\.emit\(\s*&?run_id\s*,\s*"([a-z0-9_]+)"')
STRUCT_KIND = re.compile(r'RunEvent(?:View)?\s*\{.*?\bkind:\s*"([a-z0-9_]+)"', re.S)


def docker_kinds() -> set[str]:
    """Resolve ``ALL_KINDS`` without importing the package.

    Importing ``mix_agent`` would make this zero-dependency repository gate
    require the server's virtualenv; the file is a flat list of string
    constants and one frozenset, so the AST is enough.
    """
    tree = ast.parse(DOCKER_EVENTS.read_text(encoding="utf-8"))
    constants: dict[str, str] = {}
    for node in tree.body:
        if (
            isinstance(node, ast.Assign)
            and len(node.targets) == 1
            and isinstance(node.targets[0], ast.Name)
            and isinstance(node.value, ast.Constant)
            and isinstance(node.value.value, str)
        ):
            constants[node.targets[0].id] = node.value.value

    for node in tree.body:
        if not (
            isinstance(node, ast.Assign)
            and len(node.targets) == 1
            and isinstance(node.targets[0], ast.Name)
            and node.targets[0].id == "ALL_KINDS"
        ):
            continue
        value = node.value
        if not (
            isinstance(value, ast.Call)
            and isinstance(value.func, ast.Name)
            and value.func.id == "frozenset"
            and value.args
        ):
            raise SystemExit("ALL_KINDS is no longer a frozenset(...) literal; update the extractor.")
        literal = value.args[0]
        if not isinstance(literal, (ast.Set, ast.List, ast.Tuple)):
            raise SystemExit("ALL_KINDS no longer contains a set literal; update the extractor.")
        kinds: set[str] = set()
        for element in literal.elts:
            if isinstance(element, ast.Name) and element.id in constants:
                kinds.add(constants[element.id])
            elif isinstance(element, ast.Constant) and isinstance(element.value, str):
                kinds.add(element.value)
            else:
                raise SystemExit(f"cannot resolve {ast.dump(element)} inside ALL_KINDS")
        return kinds
    raise SystemExit(f"ALL_KINDS not found in {DOCKER_EVENTS.relative_to(ROOT)}")


def desktop_kinds() -> set[str]:
    engine_files: list[Path] = []
    for target in DESKTOP_ENGINE:
        engine_files.extend(sorted(target.rglob("*.rs")) if target.is_dir() else [target])

    kinds: set[str] = set()
    for path in engine_files:
        if not path.exists():
            raise SystemExit(f"missing Desktop source: {path.relative_to(ROOT)}")
        text = path.read_text(encoding="utf-8")
        kinds.update(match.group(1) for match in EMIT_CALL.finditer(text))
        # A dynamic kind would be invisible to the check above. Fail loudly so
        # the extractor is extended instead of silently losing a kind.
        for point in EMIT_POINT.finditer(text):
            if not EMIT_CALL.match(text, point.start()):
                line = text[: point.start()].count("\n") + 1
                raise SystemExit(
                    f"{path.relative_to(ROOT)}:{line}: .emit() kind is not a string literal; "
                    "use a literal or extend scripts/check-run-event-parity.py"
                )

    # Command-layer emissions (`engine.emit(run_id, ...)`) are run events;
    # `app.emit("local_job_event", ...)` is not, and its literal first argument
    # never matches RUN_ID_EMIT.
    for path in sorted(DESKTOP_APP.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        kinds.update(match.group(1) for match in RUN_ID_EMIT_CALL.finditer(text))
        for point in RUN_ID_EMIT.finditer(text):
            if not RUN_ID_EMIT_CALL.match(text, point.start()):
                line = text[: point.start()].count("\n") + 1
                raise SystemExit(
                    f"{path.relative_to(ROOT)}:{line}: run-event kind is not a string literal; "
                    "use a literal or extend scripts/check-run-event-parity.py"
                )

    for path in DESKTOP_MARKERS:
        if not path.exists():
            raise SystemExit(f"missing Desktop source: {path.relative_to(ROOT)}")
        kinds.update(match.group(1) for match in STRUCT_KIND.finditer(path.read_text(encoding="utf-8")))
    return kinds


def describe(prefix: str, present: set[str], expected: set[str]) -> list[str]:
    lines = []
    for kind in sorted(present - expected):
        lines.append(f"  {prefix} emits but the contract does not list it: {kind}")
    for kind in sorted(expected - present):
        lines.append(f"  the contract lists a {prefix} kind that is not emitted: {kind}")
    return lines


def main() -> int:
    contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
    shared = set(contract["shared"])
    docker_only = set(contract["docker_only"])
    desktop_only = set(contract["desktop_only"])

    problems: list[str] = []
    if shared & docker_only or shared & desktop_only or docker_only & desktop_only:
        problems.append("  a kind appears in more than one contract section")
    if len(contract["shared"]) != len(shared):
        problems.append("  `shared` contains a duplicate")

    actual_docker = docker_kinds()
    actual_desktop = desktop_kinds()
    problems.extend(describe("Docker", actual_docker, shared | docker_only))
    problems.extend(describe("Desktop", actual_desktop, shared | desktop_only))

    if problems:
        print(f"Run event contract mismatch ({CONTRACT.relative_to(ROOT)}):", file=sys.stderr)
        for line in problems:
            print(line, file=sys.stderr)
        print(
            "\nAdd the kind to `shared` when both editions emit it; otherwise list it in "
            "docker_only/desktop_only with the reason.",
            file=sys.stderr,
        )
        return 1

    print(
        "Run event contract OK: "
        f"{len(shared)} shared, {len(docker_only)} Docker-only, "
        f"{len(desktop_only)} Desktop-only "
        f"({len(actual_docker)} Docker / {len(actual_desktop)} Desktop total)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
