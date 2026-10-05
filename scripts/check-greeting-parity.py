#!/usr/bin/env python3
"""Cross-edition consistency check for the chat greeting.

``desktop/`` and ``docker/`` deliberately share no code (see the root README),
so the greeting pools, their id-keyed English maps and the suggestion text are
duplicated by hand. That duplication drifts silently: a fix applied to one
edition (a typo, a new fragment, a translation) is easily forgotten in the
other, and a fragment without an English entry silently falls back to Japanese.

This script fails (exit status 1) when:

  * a fragment id present in both editions has a different Japanese text,
    except for the vocabulary differences listed in ``ALLOWED_DIVERGENCE``
  * a fragment has no English entry in its edition's ``translations.ts``
  * a greeting suggestion label/prompt has no English entry
  * a fragment id or Japanese text is duplicated inside one edition
  * a shared fragment id has a different English text

Usage::

    python3 scripts/check-greeting-parity.py

It is also wired into ``.github/workflows/ci.yml``.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

EDITIONS = {
    "desktop": {
        "pools": ROOT / "desktop/mix-agent-desktop/ui/src/features/chat/greeting/pools.ts",
        "compose": ROOT / "desktop/mix-agent-desktop/ui/src/features/chat/greeting/compose.ts",
        "suggestions": ROOT / "desktop/mix-agent-desktop/ui/src/features/chat/greeting/suggestions.ts",
        "translations": ROOT / "desktop/mix-agent-desktop/ui/src/features/chat/greeting/translations.ts",
    },
    "docker": {
        "pools": ROOT / "docker/MIX-agent/apps/web/src/features/chat/greeting/pools.ts",
        "compose": ROOT / "docker/MIX-agent/apps/web/src/features/chat/greeting/compose.ts",
        "suggestions": ROOT / "docker/MIX-agent/apps/web/src/features/chat/greeting/suggestions.ts",
        "translations": ROOT / "docker/MIX-agent/apps/web/src/features/chat/greeting/translations.ts",
    },
}

# Shared fragment ids that intentionally differ in wording between the editions:
# the Desktop edition says "スレッド" where the Docker edition says "会話"
# (see desktop/README.md), which changes both the Japanese and English text.
ALLOWED_DIVERGENCE = {"ac-1", "wk-3"}

SENTENCE = re.compile(r'sentence\(\s*"([^"]*)"\s*,\s*"((?:[^"\\]|\\.)*)"')
LABEL = re.compile(r'label:\s*"((?:[^"\\]|\\.)*)"')
PROMPT = re.compile(r'prompt:\s*"((?:[^"\\]|\\.)*)"')
SUGGESTION_ID = re.compile(r'id:\s*"(sg-[^"]*)"')
TRANSLATION = re.compile(r'"((?:[^"\\]|\\.)*)"\s*:\s*"((?:[^"\\]|\\.)*)"')


def unescape(raw: str) -> str:
    return raw.replace('\\"', '"').replace("\\\\", "\\")


def parse_fragments(paths: dict[str, Path]) -> tuple[dict[str, str], list[str]]:
    """id -> Japanese text, from the pools and the compose fallback.

    The compose fallback text legitimately repeats a pools fragment, so the
    duplicate check runs against the pools-only texts returned alongside.
    """
    fragments: dict[str, str] = {}
    pool_texts: list[str] = []
    for key in ("pools", "compose"):
        text = paths[key].read_text(encoding="utf-8")
        for match in SENTENCE.finditer(text):
            fragments[match.group(1)] = unescape(match.group(2))
            if key == "pools":
                pool_texts.append(unescape(match.group(2)))
    return fragments, pool_texts


def parse_suggestions(path: Path) -> dict[str, str]:
    """`<id>.label` / `<id>.prompt` -> Japanese text."""
    source = path.read_text(encoding="utf-8")
    suggestions: dict[str, str] = {}
    for block in re.split(r'(?=id:\s*"sg-)', source):
        match = SUGGESTION_ID.search(block)
        if not match:
            continue
        label = LABEL.search(block)
        prompt = PROMPT.search(block)
        if label:
            suggestions[f"{match.group(1)}.label"] = unescape(label.group(1))
        if prompt:
            suggestions[f"{match.group(1)}.prompt"] = unescape(prompt.group(1))
    return suggestions


def parse_translations(path: Path) -> dict[str, str]:
    return {
        unescape(key): unescape(value)
        for key, value in TRANSLATION.findall(path.read_text(encoding="utf-8"))
    }


def main() -> int:
    problems: list[str] = []
    editions: dict[str, dict[str, dict[str, str]]] = {}

    for name, paths in EDITIONS.items():
        fragments, pool_texts = parse_fragments(paths)
        suggestions = parse_suggestions(paths["suggestions"])
        translations = parse_translations(paths["translations"])

        for key, text in {**fragments, **suggestions}.items():
            if key not in translations:
                problems.append(f"{name}: no English entry for {key}: {text!r}")

        for text in sorted({t for t in pool_texts if pool_texts.count(t) > 1}):
            problems.append(f"{name}: duplicated fragment text: {text!r}")

        editions[name] = {"fragments": fragments, "translations": translations}

    desktop = editions["desktop"]
    docker = editions["docker"]

    for fragment_id in sorted(set(desktop["fragments"]) & set(docker["fragments"])):
        if fragment_id in ALLOWED_DIVERGENCE:
            continue
        if desktop["fragments"][fragment_id] != docker["fragments"][fragment_id]:
            problems.append(
                f"shared fragment {fragment_id} differs (ja): "
                f"desktop={desktop['fragments'][fragment_id]!r} "
                f"docker={docker['fragments'][fragment_id]!r}"
            )
        if desktop["translations"].get(fragment_id) != docker["translations"].get(fragment_id):
            problems.append(
                f"shared fragment {fragment_id} differs (en): "
                f"desktop={desktop['translations'].get(fragment_id)!r} "
                f"docker={docker['translations'].get(fragment_id)!r}"
            )

    print(
        f"desktop fragments: {len(desktop['fragments'])}  "
        f"docker fragments: {len(docker['fragments'])}  "
        f"shared: {len(set(desktop['fragments']) & set(docker['fragments']))}  "
        f"desktop-only: {len(set(desktop['fragments']) - set(docker['fragments']))}"
    )

    if problems:
        print("\nGreeting parity check failed:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1

    print("Greeting parity check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
