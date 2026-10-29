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
    """
    budgets = {
        "routes.py": 3555,
        "routes_auth.py": 245,
        "routes_notifications.py": 145,
        "routes_providers.py": 325,
        "routes_artifacts.py": 100,
    }
    offenders = {}
    for path in API_DIR.glob("routes*.py"):
        lines = len(path.read_text(encoding="utf-8").splitlines())
        limit = budgets.get(path.name, 400)
        if lines > limit:
            offenders[path.name] = f"{lines} > {limit}"
    assert not offenders, f"Split a new API module instead of growing {offenders}"
