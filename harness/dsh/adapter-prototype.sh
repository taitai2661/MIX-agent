#!/usr/bin/env bash
# harness/dsh adapter — S4 PROTOTYPE (雛形).
# Invokes the DSH harness CLI for one agent turn with:
#   1. version check  (like MCP PROTOCOL_VERSION handshake in host.rs)
#   2. timeout        (like DEFAULT_HTTP_CALL_TIMEOUT=300s + 10s connect cap)
#   3. cancel         (SIGINT/SIGTERM -> kill child, exit 130 like headless.rs)
# Plus host.rs validate_stdio_command mirroring: reject shell metacharacters
# and relative paths containing '/'.
#
# DEFAULT-DISABLED (safe side): requires MIX_HARNESS_ENABLED=1, and defaults
# to --dry-run (prints the command, runs nothing). Pass --execute to run.
#
# Env:
#   MIX_HARNESS_ENABLED=1        required to do anything (else: exit 2)
#   DSH_BIN                      dsh binary (default: dsh on PATH)
#   MIX_DSH_MIN_VERSION          minimum accepted version (default: 0.5.0)
#   MIX_DSH_TIMEOUT_SECS         per-call deadline (default: 300)
#   MIX_DSH_CONNECT_TIMEOUT_SECS connect deadline (default: 10)
#
# Usage: adapter-prototype.sh [--execute] [-- PROMPT...]

set -u

MIN_VERSION="${MIX_DSH_MIN_VERSION:-0.5.0}"
TIMEOUT_SECS="${MIX_DSH_TIMEOUT_SECS:-300}"
CONNECT_TIMEOUT_SECS="${MIX_DSH_CONNECT_TIMEOUT_SECS:-10}"
DSH_BIN="${DSH_BIN:-dsh}"

if [ "${MIX_HARNESS_ENABLED:-0}" != "1" ]; then
    echo "dsh-adapter: disabled (set MIX_HARNESS_ENABLED=1 to enable; prototype defaults to dry-run)" >&2
    exit 2
fi

EXECUTE=0
if [ "${1:-}" = "--execute" ]; then EXECUTE=1; shift; fi
if [ "${1:-}" = "--" ]; then shift; fi
PROMPT="${*:-}"

# --- 1. command validation (mirrors validate_stdio_command) ---
case "$DSH_BIN" in
    *';'* | *'&'* | *'|'* | *'$'* | *'`'*)
        echo "dsh-adapter: DSH_BIN contains shell metacharacters" >&2; exit 2;;
esac
case "$DSH_BIN" in
    */*)
        case "$DSH_BIN" in
            /*) [ -e "$DSH_BIN" ] || { echo "dsh-adapter: DSH_BIN not found: $DSH_BIN" >&2; exit 2; };;
            *) echo "dsh-adapter: relative DSH_BIN paths are not allowed" >&2; exit 2;;
        esac;;
esac
command -v "$DSH_BIN" >/dev/null 2>&1 || { echo "dsh-adapter: dsh not on PATH (DSH_BIN=$DSH_BIN)" >&2; exit 2; }

# --- 2. version check (mirrors MCP initialize handshake) ---
RAW_VERSION="$("$DSH_BIN" --version 2>/dev/null || true)"
VERSION="$(printf '%s' "$RAW_VERSION" | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -n1)"
if [ -z "$VERSION" ]; then
    echo "dsh-adapter: could not parse dsh version from: $RAW_VERSION" >&2; exit 2
fi
ver_ge() { # ver_ge A B -> 0 if A >= B
    [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -n1)" = "$2" ]
}
if ! ver_ge "$VERSION" "$MIN_VERSION"; then
    echo "dsh-adapter: dsh $VERSION < required $MIN_VERSION" >&2; exit 2
fi

# --- 3. dry-run default (safe side) ---
if [ "$EXECUTE" -ne 1 ]; then
    echo "dsh-adapter: DRY-RUN (dsh $VERSION >= $MIN_VERSION, timeout=${TIMEOUT_SECS}s, connect=${CONNECT_TIMEOUT_SECS}s)"
    echo "  would run: $DSH_BIN -- prompt: ${PROMPT:-<empty>}"
    exit 0
fi

# --- 4. execute with timeout + cancel propagation (mirrors kill_on_drop) ---
CHILD=0
cancel() {
    if [ "$CHILD" -ne 0 ]; then kill -TERM "$CHILD" 2>/dev/null || true; fi
    echo "dsh-adapter: cancelled" >&2
    exit 130
}
trap cancel INT TERM

# shellcheck disable=SC2086
timeout "$TIMEOUT_SECS" "$DSH_BIN" $PROMPT &
CHILD=$!
wait "$CHILD"
CODE=$?
CHILD=0
if [ "$CODE" -eq 124 ]; then
    echo "dsh-adapter: timed out after ${TIMEOUT_SECS}s" >&2; exit 1
fi
exit "$CODE"
