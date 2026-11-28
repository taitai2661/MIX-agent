"""Layering guards for the Docker edition.

The API layer may depend on the run engine, but the engine must not depend on
the API: that inversion is what made `finish()` import a function that lived in
`api.routes`, and a temporary run then failed at runtime with an ImportError.
"""

import ast
from pathlib import Path

import mix_agent

SERVER = Path(mix_agent.__file__).resolve().parent
API_DIR = SERVER / "api"


def _imports(path: Path) -> set[str]:
    tree = ast.parse(path.read_text(encoding="utf-8"))
    found: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            found.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module:
            found.add(node.module)
    return found


def test_run_engine_does_not_import_the_api_layer():
    offenders = {}
    for path in (SERVER / "runs").rglob("*.py"):
        bad = sorted(name for name in _imports(path) if name.startswith("mix_agent.api"))
        if bad:
            offenders[str(path.relative_to(SERVER))] = bad
    assert not offenders, f"runs/* must not import the API layer: {offenders}"


def test_storage_helpers_do_not_import_the_api_layer():
    offenders = {}
    for path in (SERVER / "storage").rglob("*.py"):
        bad = sorted(name for name in _imports(path) if name.startswith("mix_agent.api"))
        if bad:
            offenders[str(path.relative_to(SERVER))] = bad
    assert not offenders, f"storage/* must not import the API layer: {offenders}"


def test_route_modules_do_not_grow_unbounded():
    """Per-router line budgets, not one number for every module.

    A single 3,650-line limit left ``routes.py`` only 40 lines of headroom, so
    the next endpoint failed CI and said nothing about the small modules.  Each
    router now has its own budget: when one is reached, extract the next
    cluster instead of raising the number.

    The numbers are ceilings *with headroom* (about 15 lines over the measured
    size), not exact snapshots: a trivial edit or a log line must not break an
    unrelated PR. They are still a ratchet — lower a number when a router
    shrinks, and extract a module when the ceiling is genuinely approached.
    """
    budgets = {
        # Ceilings measured at the last refactor plus headroom; see the ratchet
        # rule in the docstring. `routes_messages` is the largest and the next
        # candidate to split.
        "routes.py": 53,
        "routes_artifacts.py": 75,
        # +1: request_client_ip keeps login throttling correct behind a proxy.
        # +1: /auth/me returns 401 when the user row is gone.
        "routes_auth.py": 221,
        # +6: title_is_placeholder records user-provided titles explicitly.
        "routes_collections.py": 263,
        "routes_conversations.py": 446,
        # +13: MCP install/sync never reuse a closed request session; fresh
        # SessionLocal for post-I/O work plus orphan-secret cleanup.
        "routes_mcp.py": 492,
        "routes_memories.py": 221,
        # Idempotency lookup and tool selection moved to
        # message_idempotency.py / message_tools.py.
        "routes_messages.py": 400,
        "routes_notifications.py": 120,
        # ChatGPT-specific HTTP lives in routes_chatgpt.py; this module keeps
        # one small normalization branch for the preset and its skip-sync rule.
        "routes_providers.py": 325,
        "routes_runs.py": 553,
        "routes_schedules.py": 152,
        "routes_settings.py": 473,
        "routes_skills.py": 190,
        "routes_statistics.py": 385,
        # Extracted from routes_messages so the router can stay a thin HTTP
        # surface. Keep them from quietly growing back into a second monolith.
        "message_context.py": 283,
        "message_runs.py": 196,
    }
    offenders = {}
    for path in [*API_DIR.glob("routes*.py"), *API_DIR.glob("message_*.py")]:
        lines = len(path.read_text(encoding="utf-8").splitlines())
        limit = budgets.get(path.name, 400)
        if lines > limit:
            offenders[path.name] = f"{lines} > {limit}"
    assert not offenders, f"Split a new API module instead of growing {offenders}"
