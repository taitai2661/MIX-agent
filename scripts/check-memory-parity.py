#!/usr/bin/env python3
"""Cross-edition consistency check for the Memory contract.

``desktop/`` and ``docker/`` deliberately share no code (see the root README),
so the Memory spec is duplicated by hand in each edition, and each edition
keeps its own copy of the vocabulary (roles / scopes / lifecycle states /
relation kinds). That duplication drifts silently: a state added on one side
is forgotten on the other, and the two editions then disagree about what a
stored Memory row means.

This script fails (exit status 1) when:

  * the two ``memory-spec.md`` files differ in any byte
  * either spec is missing a required section heading
  * the machine-readable ``json vocabulary`` block is absent or not valid JSON
  * a vocabulary list in the spec does not match the constants the edition
    actually defines (Python: ``memory/types.py``, Rust: ``core/src/memory.rs``)
  * a number in the spec's ``ranking`` block does not match the ranking
    constants each edition computes with (Python: ``memory/service.py``,
    Rust: ``core/src/retrieval.rs``)
  * the shared extraction prompt in spec §6 is not reproduced verbatim by each
    edition (Python: ``memory/jobs.py``, Rust: ``core/src/memory.rs``)
  * the shared recall fixture differs in any byte between the two editions
    (``tests/fixtures/memory_recall_cases.json`` in each tree)

The ranking numbers are checked because they are the part of the contract a
user can feel: two editions that weight lexical, vector and recency differently
retrieve different memories from identical stores, and nothing in either
codebase would notice. The extraction prompt is checked for the same reason:
two editions that ask different questions extract different memories, and the
drift is invisible from either side.

Usage::

    python3 scripts/check-memory-parity.py

It is also wired into ``.github/workflows/ci.yml`` (job ``memory-parity``).
"""

from __future__ import annotations

import ast
import json
import math
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

SPEC_PATHS = {
    "docker": ROOT / "docs/memory-spec.md",
    "desktop": ROOT / "docs/desktop/memory-spec.md",
}

IMPL_PATHS = {
    "docker": ROOT / "apps/server/mix_agent/memory/types.py",
    "desktop": ROOT / "apps/desktop/core/src/memory.rs",
}

RANKING_PATHS = {
    "docker": ROOT / "apps/server/mix_agent/memory/service.py",
    "desktop": ROOT / "apps/desktop/core/src/retrieval.rs",
}

# spec key -> constant name, per edition.
PYTHON_CONSTANTS = {
    "roles": "ROLES",
    "scopes": "SCOPES",
    "sources": "SOURCES",
    "verifications": "VERIFICATIONS",
    "lifecycles": "LIFECYCLES",
    "relation_kinds": "RELATION_KINDS",
    "retrievable_lifecycles": "LIFECYCLE_RETRIEVABLE",
}

RUST_CONSTANTS = {
    "roles": "ROLE_NAMES",
    "scopes": "SCOPE_NAMES",
    "sources": "SOURCE_KINDS",
    "verifications": "VERIFICATION_STATES",
    "lifecycles": "LIFECYCLE_STATES",
    "relation_kinds": "RELATION_KINDS",
    "retrievable_lifecycles": "RETRIEVABLE_LIFECYCLES",
}

# spec ``ranking`` key -> constant name. The Desktop edition prefixes the one
# memory-specific limit so it cannot be confused with the knowledge clip.
RANKING_CONSTANTS = {
    "docker": {
        "lexical_weight_with_vector": "LEXICAL_WEIGHT_WITH_VECTOR",
        "vector_weight_with_vector": "VECTOR_WEIGHT_WITH_VECTOR",
        "recency_weight_with_vector": "RECENCY_WEIGHT_WITH_VECTOR",
        "lexical_weight_without_vector": "LEXICAL_WEIGHT_WITHOUT_VECTOR",
        "recency_weight_without_vector": "RECENCY_WEIGHT_WITHOUT_VECTOR",
        "min_score": "MIN_SCORE",
        "result_limit_default": "RESULT_LIMIT_DEFAULT",
        "result_limit_max": "RESULT_LIMIT_MAX",
        "item_char_limit": "ITEM_CHAR_LIMIT",
    },
    "desktop": {
        "lexical_weight_with_vector": "LEXICAL_WEIGHT_WITH_VECTOR",
        "vector_weight_with_vector": "VECTOR_WEIGHT_WITH_VECTOR",
        "recency_weight_with_vector": "RECENCY_WEIGHT_WITH_VECTOR",
        "lexical_weight_without_vector": "LEXICAL_WEIGHT_WITHOUT_VECTOR",
        "recency_weight_without_vector": "RECENCY_WEIGHT_WITHOUT_VECTOR",
        "min_score": "MIN_SCORE",
        "result_limit_default": "RESULT_LIMIT_DEFAULT",
        "result_limit_max": "RESULT_LIMIT_MAX",
        "item_char_limit": "MEMORY_ITEM_CHAR_LIMIT",
    },
}

# Sections that must exist in both specs. Dropping one silently un-contracts it.
REQUIRED_HEADINGS = (
    "## 0. 位置づけ",
    "## 1. データモデル",
    "## 2. 語彙（正）",
    "## 3. ライフサイクル遷移",
    "## 4. `memory_links`（関連・上書き・矛盾）",
    "## 5. 検索とランキング",
    "## 6. 自動形成（post-response 抽出）",
    "## 7. tool 契約",
    "## 8. 秘密情報の扱い",
    "## 9. 本仕様に対して未実装 / 不整合なもの",
)

# Keys that are contractual but not (yet) mirrored as code constants.
UNCHECKED_KEYS = ("tool_actions",)

# Spec §6 publishes the extraction prompt so both editions ask the model the
# same question. Each edition must reproduce it verbatim as a module-level
# `EXTRACTION_PROMPT`: a triple-quoted string in Python, a `r#"..."#` raw
# string in Rust (raw precisely so the text is copy-pasteable without escapes).
EXTRACTION_PROMPT_BLOCK = re.compile(r"\*\*共通抽出プロンプト\*\*:\s*\n\n```\n(.*?)\n```", re.DOTALL)

PROMPT_PATHS = {
    "docker": ROOT / "apps/server/mix_agent/memory/jobs.py",
    "desktop": ROOT / "apps/desktop/core/src/memory.rs",
}

# The ranking cases pin the numbers both editions must produce. Like the spec,
# one copy lives in each repository so each stays cloneable on its own; this
# script is what makes the copies mean the same thing.
FIXTURE_PATHS = {
    "docker": ROOT / "tests/fixtures/memory_recall_cases.json",
    "desktop": ROOT / "apps/desktop/core/tests/fixtures/memory_recall_cases.json",
}

VOCAB_BLOCK = re.compile(r"```json vocabulary\n(.*?)\n```", re.DOTALL)


def fail(message: str) -> None:
    print(f"FAIL: {message}")


def read_spec(edition: str) -> str:
    path = SPEC_PATHS[edition]
    if not path.is_file():
        raise SystemExit(f"FAIL: missing spec for {edition}: {path}")
    return path.read_text(encoding="utf-8")


def vocabulary_of(text: str, edition: str) -> dict:
    match = VOCAB_BLOCK.search(text)
    if not match:
        raise SystemExit(f"FAIL: {edition} spec has no ```json vocabulary block")
    try:
        payload = json.loads(match.group(1))
    except json.JSONDecodeError as exc:
        raise SystemExit(f"FAIL: {edition} vocabulary block is not valid JSON: {exc}") from exc
    if not isinstance(payload, dict):
        raise SystemExit(f"FAIL: {edition} vocabulary block is not a JSON object")
    return payload


#: key -> (values, ordered). ``frozenset`` literals have no meaningful order
#: (string hashing is salted per process), so they compare as sets.
ConstantList = tuple[list[str], bool]


def python_constants(path: Path, wanted: dict[str, str]) -> dict[str, ConstantList]:
    """Read module-level constants with ``ast`` so reformatting cannot hide them."""
    found = _module_constants(path)

    # ``ROLES`` is written as ``(ROLE_FACT, ROLE_DECISION, ...)``, so plain
    # ``literal_eval`` cannot see through it: resolve single-name constants
    # first and substitute them while walking the collection.
    strings = {
        name: value.value
        for name, value in found.items()
        if isinstance(value, ast.Constant) and isinstance(value.value, str)
    }

    out: dict[str, ConstantList] = {}
    for key, name in wanted.items():
        node = found.get(name)
        if node is None:
            raise SystemExit(f"FAIL: {path.name} does not define {name}")
        out[key] = _literal_list(node, strings, f"{path.name}:{name}")
    return out


def python_numbers(path: Path, wanted: dict[str, str]) -> dict[str, float]:
    """Read module-level numeric constants (the ranking contract)."""
    found = _module_constants(path)
    out: dict[str, float] = {}
    for key, name in wanted.items():
        node = found.get(name)
        if (
            not isinstance(node, ast.Constant)
            or not isinstance(node.value, (int, float))
            or isinstance(node.value, bool)
        ):
            raise SystemExit(f"FAIL: {path.name} does not define {name} as a numeric literal")
        out[key] = float(node.value)
    return out


def _module_constants(path: Path) -> dict[str, ast.AST]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    found: dict[str, ast.AST] = {}
    for node in tree.body:
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            found[node.target.id] = node.value
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    found[target.id] = node.value
    return found


def _literal_list(node: ast.AST, strings: dict[str, str], where: str) -> ConstantList:
    # ``frozenset({...})`` / ``set({...})`` / ``[...]`` / ``(...)`` all work.
    unordered = False
    if isinstance(node, ast.Call):
        if not isinstance(node.func, ast.Name) or node.func.id not in {"frozenset", "set", "list", "tuple"}:
            raise SystemExit(f"FAIL: {where} is not a plain collection literal")
        if not node.args:
            raise SystemExit(f"FAIL: {where} is empty")
        unordered = node.func.id in {"frozenset", "set"}
        node = node.args[0]
    if isinstance(node, (ast.Tuple, ast.List, ast.Set)):
        items = [_resolve_name(element, strings, where) for element in node.elts]
    elif isinstance(node, ast.Constant) and isinstance(node.value, str):
        items = [node.value]
    else:
        raise SystemExit(f"FAIL: {where} is not a collection literal")
    if not items or any(not item for item in items):
        raise SystemExit(f"FAIL: {where} has an empty entry")
    return (sorted(items) if unordered else items), not unordered


def _resolve_name(node: ast.AST, strings: dict[str, str], where: str) -> str:
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    if isinstance(node, ast.Name) and node.id in strings:
        return strings[node.id]
    raise SystemExit(f"FAIL: {where} references a value that is not a string literal: {ast.dump(node)}")


def rust_constants(path: Path, wanted: dict[str, str]) -> dict[str, ConstantList]:
    text = path.read_text(encoding="utf-8")
    out: dict[str, ConstantList] = {}
    for key, name in wanted.items():
        pattern = re.compile(
            rf"pub\s+const\s+{re.escape(name)}\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\]\s*;",
            re.DOTALL,
        )
        match = pattern.search(text)
        if not match:
            raise SystemExit(f"FAIL: {path.name} does not define {name}")
        items = re.findall(r'"([^"]*)"', match.group(1))
        if not items:
            raise SystemExit(f"FAIL: {path.name}:{name} is empty")
        out[key] = (items, True)
    return out


def rust_numbers(path: Path, wanted: dict[str, str]) -> dict[str, float]:
    """Read ``pub const NAME: f64|usize = 1.5;`` ranking constants."""
    text = path.read_text(encoding="utf-8")
    out: dict[str, float] = {}
    for key, name in wanted.items():
        pattern = re.compile(
            rf"pub\s+const\s+{re.escape(name)}\s*:\s*(?:f64|usize|u32)\s*=\s*([0-9]+(?:\.[0-9]+)?)\s*;"
        )
        match = pattern.search(text)
        if not match:
            raise SystemExit(f"FAIL: {path.name} does not define {name} as a numeric constant")
        out[key] = float(match.group(1))
    return out


def spec_extraction_prompt(text: str) -> str:
    match = EXTRACTION_PROMPT_BLOCK.search(text)
    if not match:
        raise SystemExit("FAIL: spec has no 共通抽出プロンプト block in §6")
    return match.group(1).strip()


def python_extraction_prompt(path: Path) -> str:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(
            isinstance(t, ast.Name) and t.id == "EXTRACTION_PROMPT" for t in node.targets
        ):
            if isinstance(node.value, ast.Constant) and isinstance(node.value.value, str):
                return node.value.value.strip()
    raise SystemExit(f"FAIL: {path.name} does not define EXTRACTION_PROMPT as a string literal")


def rust_extraction_prompt(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    match = re.search(r'pub\s+const\s+EXTRACTION_PROMPT\s*:\s*&str\s*=\s*r#"(.*?)"#\s*;', text, re.DOTALL)
    if not match:
        raise SystemExit(f'FAIL: {path.name} does not define EXTRACTION_PROMPT as a r#"..."# literal')
    return match.group(1).strip()


def check_extraction_prompt(expected: str, edition: str, actual: str, errors: list[str]) -> None:
    if actual == expected:
        return
    expected_lines = expected.splitlines()
    actual_lines = actual.splitlines()
    for index, (left, right) in enumerate(zip(expected_lines, actual_lines), start=1):
        if left != right:
            errors.append(
                f"{edition} extraction prompt diverges from spec §6 at line {index}: {left!r} != {right!r}"
            )
            break
    else:
        errors.append(
            f"{edition} extraction prompt differs in length from spec §6: "
            f"spec has {len(expected_lines)} lines, implementation has {len(actual_lines)}"
        )


def check_ranking(spec: dict, actual: dict[str, float], edition: str, errors: list[str]) -> None:
    """The ranking numbers are the contract a user can feel; drift them and the two editions retrieve different memories from the same store."""
    declared = spec.get("ranking")
    if not isinstance(declared, dict):
        errors.append("vocabulary block has no 'ranking' object")
        return
    for key, expected in sorted(declared.items()):
        if not isinstance(expected, (int, float)) or isinstance(expected, bool):
            errors.append(f"ranking {key!r} is not a number")
            continue
        if key not in actual:
            errors.append(f"{edition} ranking: spec declares {key!r} but no constant checks it")
            continue
        if not math.isclose(actual[key], float(expected), rel_tol=0.0, abs_tol=1e-9):
            errors.append(f"{edition} ranking {key}: spec says {expected}, implementation says {actual[key]}")
    for key in sorted(actual):
        if key not in declared:
            errors.append(f"{edition} ranking {key}: implemented but not declared in the spec")


def check_headings(edition: str, text: str, errors: list[str]) -> None:
    for heading in REQUIRED_HEADINGS:
        if heading not in text:
            errors.append(f"{edition} spec is missing the section: {heading}")


def check_vocabulary(edition: str, spec: dict, actual: dict[str, ConstantList], errors: list[str]) -> None:
    for key, (expected, ordered) in sorted(actual.items()):
        if key not in spec:
            errors.append(f"vocabulary block has no {key!r} (needed to check {edition})")
            continue
        declared = spec[key]
        if not isinstance(declared, list):
            errors.append(f"vocabulary {key!r} is not a list")
            continue
        declared = [str(item) for item in declared]
        # Unordered collections (frozenset) are compared as sets; their source
        # order depends on the process hash seed and means nothing.
        if declared == expected or (not ordered and sorted(declared) == expected):
            continue
        missing = [item for item in declared if item not in expected]
        extra = [item for item in expected if item not in declared]
        detail = []
        if missing:
            detail.append(f"declared but not implemented: {missing}")
        if extra:
            detail.append(f"implemented but not declared: {extra}")
        if not detail:
            detail.append(f"order differs ({declared} vs {expected})")
        errors.append(f"{edition} {key}: " + "; ".join(detail))


def main() -> int:
    errors: list[str] = []

    texts = {edition: read_spec(edition) for edition in SPEC_PATHS}
    if texts["docker"] != texts["desktop"]:
        docker_lines = texts["docker"].splitlines()
        desktop_lines = texts["desktop"].splitlines()
        for index, (left, right) in enumerate(zip(docker_lines, desktop_lines), start=1):
            if left != right:
                errors.append(f"specs diverge at line {index}: {left!r} != {right!r}")
                break
        else:
            errors.append(
                f"specs diverge in length: docker has {len(docker_lines)} lines, "
                f"desktop has {len(desktop_lines)} lines"
            )

    vocabulary = vocabulary_of(texts["docker"], "docker")
    for key in UNCHECKED_KEYS:
        if key not in vocabulary:
            errors.append(f"vocabulary block has no {key!r}")

    for edition, text in texts.items():
        check_headings(edition, text, errors)

    readers = {
        "docker": lambda: python_constants(IMPL_PATHS["docker"], PYTHON_CONSTANTS),
        "desktop": lambda: rust_constants(IMPL_PATHS["desktop"], RUST_CONSTANTS),
    }
    for edition, read in readers.items():
        try:
            actual = read()
        except SystemExit as exc:
            errors.append(str(exc).removeprefix("FAIL: "))
            continue
        check_vocabulary(edition, vocabulary, actual, errors)

    ranking_readers = {
        "docker": lambda: python_numbers(RANKING_PATHS["docker"], RANKING_CONSTANTS["docker"]),
        "desktop": lambda: rust_numbers(RANKING_PATHS["desktop"], RANKING_CONSTANTS["desktop"]),
    }
    for edition, read in ranking_readers.items():
        try:
            numbers = read()
        except SystemExit as exc:
            errors.append(str(exc).removeprefix("FAIL: "))
            continue
        check_ranking(vocabulary, numbers, edition, errors)

    try:
        prompt = spec_extraction_prompt(texts["docker"])
    except SystemExit as exc:
        errors.append(str(exc).removeprefix("FAIL: "))
        prompt = None
    if prompt is not None:
        prompt_readers = {
            "docker": lambda: python_extraction_prompt(PROMPT_PATHS["docker"]),
            "desktop": lambda: rust_extraction_prompt(PROMPT_PATHS["desktop"]),
        }
        for edition, read in prompt_readers.items():
            try:
                check_extraction_prompt(prompt, edition, read(), errors)
            except SystemExit as exc:
                errors.append(str(exc).removeprefix("FAIL: "))

    try:
        copies = {edition: path.read_bytes() for edition, path in FIXTURE_PATHS.items()}
    except OSError as exc:
        errors.append(f"cannot read the shared recall fixture: {exc}")
    else:
        first, second = copies["docker"], copies["desktop"]
        if first != second:
            errors.append(
                "recall fixture differs between editions: "
                f"docker has {len(first)} bytes, desktop has {len(second)} bytes "
                "(tests/fixtures/memory_recall_cases.json)"
            )
        else:
            try:
                parsed = json.loads(first)
                if not parsed.get("cases"):
                    errors.append("recall fixture declares no cases, so it checks nothing")
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                errors.append(f"recall fixture is not valid UTF-8 JSON: {exc}")

    if errors:
        for message in errors:
            fail(message)
        print(f"\n{len(errors)} problem(s) found.")
        return 1

    total = sum(len(vocabulary[key]) for key in vocabulary if isinstance(vocabulary[key], list))
    ranking = len(RANKING_CONSTANTS["docker"])
    print(
        f"memory parity OK: {total} vocabulary entries, {ranking} ranking constants, "
        "the extraction prompt and the recall fixture agree across both editions."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
