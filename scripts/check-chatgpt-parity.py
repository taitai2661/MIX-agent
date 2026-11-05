#!/usr/bin/env python3
"""Cross-edition contract check for the unofficial ChatGPT / Codex integration.

Desktop (Rust) and Docker (Python) each implement the same private Codex
backend independently.  The constants below have to stay byte-for-byte equal or
one edition will silently start failing (or, worse, send credentials to the
wrong host) when the other is updated.

This script fails when:

  * an OAuth / transport constant differs between the two editions
  * the token refresh skew or quota cache TTL differs
  * the device-login TTL or poll floor differs
  * the quota window thresholds used by ``_window_id`` drift
  * an explicit OAuth error code known to Python is missing from Rust

Usage::

    python3 scripts/check-chatgpt-parity.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUST_CHATGPT = ROOT / "apps/desktop/providers/src/chatgpt.rs"
RUST_AUTH = ROOT / "apps/desktop/providers/src/chatgpt/auth.rs"
RUST_QUOTA = ROOT / "apps/desktop/providers/src/chatgpt/quota.rs"
RUST_COMMANDS = ROOT / "apps/desktop/app/src/commands/chatgpt.rs"
PY_CONFIG = ROOT / "apps/server/mix_agent/providers/chatgpt_config.py"
PY_CHATGPT = ROOT / "apps/server/mix_agent/providers/chatgpt.py"

STRING_CONSTS = {
    "AUTH_ISSUER": "AUTH_ISSUER",
    "CLIENT_ID": "CLIENT_ID",
    "BASE_URL": "CHATGPT_BASE_URL",
    "USAGE_URL": "CHATGPT_USAGE_URL",
    "DEVICE_VERIFICATION_URL": "DEVICE_VERIFICATION_URL",
    "DEVICE_REDIRECT_URI": "DEVICE_REDIRECT_URI",
}

INT_CONSTS = {
    "REFRESH_SKEW_SECONDS": "REFRESH_SKEW_SECONDS",
    "QUOTA_TTL_SECONDS": "QUOTA_TTL_SECONDS",
}

RUST_CONST = re.compile(r"(?:pub\s+)?const\s+([A-Z0-9_]+)\s*:\s*[^=]+=\s*([^;]+);")
PY_CONST = re.compile(r"^([A-Z][A-Z0-9_]*)\s*=\s*(.+)$", re.M)


def fail(message: str) -> None:
    print(f"chatgpt-parity: {message}", file=sys.stderr)
    raise SystemExit(1)


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        fail(f"cannot read {path.relative_to(ROOT)}: {exc}")


def rust_consts() -> dict[str, str]:
    return {name: value.strip() for name, value in RUST_CONST.findall(read(RUST_CHATGPT))}


def py_consts() -> dict[str, str]:
    return {name: value.strip().rstrip(",") for name, value in PY_CONST.findall(read(PY_CONFIG))}


def unquote(value: str) -> str:
    match = re.fullmatch(r'["\'](.*)["\']', value, re.S)
    return match.group(1) if match else value


def py_string(name: str, py: dict[str, str]) -> str:
    """Resolve a plain or f-string constant from the Python config."""
    value = py[name].strip()
    if value.startswith(('f"', "f'")):
        value = value[1:]

        def substitute(match: re.Match[str]) -> str:
            ref = match.group(1)
            if ref not in py:
                fail(f"cannot resolve Python f-string reference {ref}")
            return py_string(ref, py)

        value = re.sub(r"\{([A-Z0-9_]+)\}", substitute, value)
    return unquote(value)


def eval_int(value: str) -> int:
    expression = value.replace("_", "")
    expression = re.sub(r"^Duration::from_secs\((.+)\)$", r"\1", expression)
    try:
        return int(eval(expression, {"__builtins__": {}}, {}))  # noqa: S307
    except Exception:  # noqa: BLE001 - any unparseable constant is a drift failure
        fail(f"cannot evaluate integer constant {value!r}")


def check_constants() -> None:
    rust = rust_consts()
    py = py_consts()
    for rust_name, py_name in STRING_CONSTS.items():
        if rust_name not in rust:
            fail(f"missing Rust constant {rust_name}")
        if py_name not in py:
            fail(f"missing Python constant {py_name}")
        rust_value = unquote(rust[rust_name])
        py_value = py_string(py_name, py)
        if rust_value != py_value:
            fail(f"{rust_name} drift: Rust={rust_value!r} Python={py_value!r}")
    for rust_name, py_name in INT_CONSTS.items():
        if rust_name not in rust:
            fail(f"missing Rust constant {rust_name}")
        if py_name not in py:
            fail(f"missing Python constant {py_name}")
        if eval_int(rust[rust_name]) != eval_int(py[py_name]):
            fail(f"{rust_name} drift: Rust={eval_int(rust[rust_name])} Python={eval_int(py[py_name])}")


def check_login_ttl() -> None:
    py = py_consts()
    rust_commands = {name: value.strip() for name, value in RUST_CONST.findall(read(RUST_COMMANDS))}
    if "LOGIN_TTL_SECONDS" not in rust_commands:
        fail("missing Rust LOGIN_TTL_SECONDS in commands/chatgpt.rs")
    if "DEVICE_TTL_SECONDS" not in py:
        fail("missing Python DEVICE_TTL_SECONDS")
    if eval_int(rust_commands["LOGIN_TTL_SECONDS"]) != eval_int(py["DEVICE_TTL_SECONDS"]):
        fail(
            "device login TTL drift: "
            f"Rust={eval_int(rust_commands['LOGIN_TTL_SECONDS'])} "
            f"Python={eval_int(py['DEVICE_TTL_SECONDS'])}"
        )

    match = re.search(r"\.max\((\d+)\)", read(RUST_AUTH))
    if not match:
        fail("cannot find the Rust device poll floor (.max(N))")
    if "LOGIN_POLL_FLOOR_SECONDS" not in py:
        fail("missing Python LOGIN_POLL_FLOOR_SECONDS")
    if int(match.group(1)) != eval_int(py["LOGIN_POLL_FLOOR_SECONDS"]):
        fail(
            "device poll floor drift: "
            f"Rust={match.group(1)} Python={eval_int(py['LOGIN_POLL_FLOOR_SECONDS'])}"
        )


def window_thresholds(source: str, label: str) -> tuple[int, int]:
    short = re.search(r"seconds\s*<=\s*(\d+)\s*\*\s*3600", source) or re.search(
        r"seconds\s*<\s*(\d+)\s*\*\s*3600", source
    )
    long = re.search(r"seconds\s*>=\s*(\d+)\s*\*\s*86400", source)
    if not short or not long:
        fail(f"cannot find quota window thresholds in {label}")
    return int(short.group(1)), int(long.group(1))


def check_windows() -> None:
    rust = window_thresholds(read(RUST_QUOTA), "quota.rs")
    py = window_thresholds(read(PY_CHATGPT), "chatgpt.py")
    if rust != py:
        fail(f"quota window thresholds drift: Rust={rust} Python={py}")


def check_refresh_codes() -> None:
    py_source = read(PY_CHATGPT)
    match = re.search(r"refresh_token_invalidated.*?\n\s*\}", py_source, re.S)
    codes = set(re.findall(r'"(refresh_token_[a-z_]+|invalid_grant)"', py_source))
    if not codes:
        fail("cannot find OAuth refresh error codes in chatgpt.py")
    rust_source = read(RUST_AUTH)
    missing = sorted(code for code in codes if code not in rust_source)
    if missing:
        fail(f"Rust refresh classification is missing: {', '.join(missing)}")
    if match is None:
        fail("cannot find the Python refresh-token error set")


def main() -> None:
    check_constants()
    check_login_ttl()
    check_windows()
    check_refresh_codes()
    print("chatgpt-parity: ok")


if __name__ == "__main__":
    main()
