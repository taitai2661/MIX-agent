#!/usr/bin/env bash
# mixd acp --help — S4 PROTOTYPE (雛形). Prints the planned CLI surface only.
# Mirrors apps/desktop/app/src/headless.rs run_help() conventions:
#   exit 0 success, 1 run failed, 2 usage error, 130 cancelled.
# Anything except --help/-h is NOT executed (safe side): prints this help
# on stderr and exits 2.
set -u

HELP_TEXT="USAGE:
    mixd acp [--transport stdio|http] [--timeout SECS] [--json] [--dry-run] [-- COMMAND...]

DESCRIPTION:
    Agent Client Protocol (ACP) bridge — prototype. Spawns or dials an ACP
    agent (e.g. harness/dsh adapter) and forwards prompts over JSON-RPC,
    mirroring the MCP client transports in apps/desktop/mcp/src/host.rs
    (stdio newline-delimited JSON-RPC / Streamable HTTP) and the headless
    'mix-agent run' policy in apps/desktop/app/src/headless.rs.

OPTIONS:
    --transport <stdio|http>  ACP transport (default: stdio)
    --timeout <SECS>          Per-call deadline, like DEFAULT_HTTP_CALL_TIMEOUT
                              (default: 300; connect phase capped at 10)
    --json                    Stream ACP events as NDJSON (like 'run --json')
    --dry-run                 Print what would run and exit 0 (default-safe)
    -h, --help                Show this help

Exit codes: 0 success, 1 run failed, 2 usage error, 130 cancelled."

case "${1:-}" in
    -h|--help|"")
        printf '%s\n' "$HELP_TEXT"
        exit 0
        ;;
    *)
        printf 'mixd acp: PROTOTYPE — not implemented (got: %s)\n\n%s\n' "$*" "$HELP_TEXT" >&2
        exit 2
        ;;
esac
