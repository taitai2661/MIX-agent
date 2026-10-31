import os
import secrets
from pathlib import Path

for directory in (
    "/keys",
    "/db-secret",
    "/execution-token",
    "/mcp-token",
    "/manager-token",
    "/browser-token",
    "/browser-provisioner-token",
    "/browser-data",
    "/data",
    "/policy",
    "/workspace",
    "/packages",
    "/shared",
):
    root = Path(directory)
    root.mkdir(parents=True, exist_ok=True)
    os.chown(root, 1000, 1000)
for filename, binary in (
    ("/keys/master.key", True),
    ("/db-secret/password", False),
    ("/execution-token/execution.token", False),
    ("/mcp-token/mcp.token", False),
    ("/manager-token/manager.token", False),
    ("/browser-token/browser.token", False),
    ("/browser-provisioner-token/browser-provisioner.token", False),
):
    file = Path(filename)
    try:
        with file.open("xb") as handle:
            handle.write(secrets.token_bytes(32) if binary else secrets.token_urlsafe(48).encode())
        os.chmod(file, 0o644 if filename.startswith("/db-secret") else 0o600)
        os.chown(file, 1000, 1000)
    except FileExistsError:
        pass
policy = Path("/policy/policy.json")
if not policy.exists():
    # Explicit `mode` instead of the legacy "empty list = allow all" inference:
    # a self-hoster sees the choice in the file and in the UI, and locking it
    # down later (`restricted`) cannot be confused with a broken policy.
    policy.write_text('{"mode": "public_web", "allowed_domains": []}')
    os.chown(policy, 1000, 1000)
# An empty MCP policy means "no per-source overrides"; the global policy still
# applies.  Creating it up front keeps a missing file distinguishable from an
# intentionally empty policy.
mcp_policy = Path("/policy/mcp-policy.json")
if not mcp_policy.exists():
    mcp_policy.write_text('{"source_policies": {}}')
    os.chown(mcp_policy, 1000, 1000)
