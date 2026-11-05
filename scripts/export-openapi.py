import json
import os
import sys
import tempfile
from pathlib import Path

from mix_agent.main import app

root = Path(__file__).resolve().parents[1]
target = root / "docs/openapi.json"
try:
    payload = json.dumps(app.openapi(), ensure_ascii=False, indent=2)
    # Write to a sibling temp file and rename so a crash never leaves a
    # truncated openapi.json behind.
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=target.parent, prefix=target.name + ".", delete=False
    ) as handle:
        handle.write(payload)
        tmp = handle.name
    os.replace(tmp, target)
except Exception as exc:  # noqa: BLE001 - report and fail non-zero for CI
    print(f"Failed to export docs/openapi.json: {exc}", file=sys.stderr)
    sys.exit(1)
print("Exported docs/openapi.json")
