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
            handle.write(
                secrets.token_bytes(32)
                if binary
                else secrets.token_urlsafe(48).encode()
            )
        os.chmod(file, 0o644 if filename.startswith("/db-secret") else 0o600)
        os.chown(file, 1000, 1000)
    except FileExistsError:
        pass
policy = Path("/policy/policy.json")
if not policy.exists():
    policy.write_text('{"allowed_domains": []}')
    os.chown(policy, 1000, 1000)
