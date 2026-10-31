"""Guards for the repository's own configuration files.

These are cheap invariants that do not need a database: the Python tooling
configuration is duplicated between the root and `apps/server` for good
reasons, so the duplication is asserted instead of left to drift.
"""

import configparser
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def test_pytest_configs_agree_on_mode_and_testpaths():
    root = configparser.ConfigParser()
    root.read(ROOT / "pytest.ini", encoding="utf-8")
    server = tomllib.loads((ROOT / "apps/server/pyproject.toml").read_text(encoding="utf-8"))

    pytest_options = server["tool"]["pytest"]["ini_options"]
    # `pythonpath` is a root-only concern (the container mounts the checkout at
    # /workspace), but both must agree about how tests run and where they live.
    assert pytest_options["asyncio_mode"] == root["pytest"]["asyncio_mode"]
    assert len(pytest_options["testpaths"]) == 1
    assert (
        Path(pytest_options["testpaths"][0]).name
        == Path(root["pytest"]["testpaths"].split(",")[0].strip()).name
    )


def test_ruff_has_a_single_configuration():
    # One config means `ruff check` behaves the same from the repo root, from
    # apps/server, and in CI. A nested `[tool.ruff]` would silently win for that
    # subtree and re-narrow the scope, which is how `runners/` went unlinted.
    assert (ROOT / "ruff.toml").exists()
    server = tomllib.loads((ROOT / "apps/server/pyproject.toml").read_text(encoding="utf-8"))
    assert "ruff" not in server.get("tool", {})


def test_ruff_covers_every_python_surface():
    # The root config alone does not prove coverage; the CI invocation decides
    # which directories are linted, so assert against that list.
    ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    ruff_line = next(line for line in ci.splitlines() if "ruff check" in line)
    for surface in ("apps/server", "runners", "deploy", "scripts", "tests"):
        assert surface in ruff_line, f"{surface} is not linted in CI"


def test_compose_test_database_has_no_committed_password():
    compose = (ROOT / "compose.yaml").read_text(encoding="utf-8")
    # The throwaway test DB uses trust auth on an internal network; a password
    # file in the repository would be a real (if low-value) credential.
    assert "test-db-password" not in compose
    assert not (ROOT / "deploy/docker/test-db-password").exists()
