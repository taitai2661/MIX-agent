"""Guards for the repository's own configuration files.

These are cheap invariants that do not need a database: the Python tooling
configuration is duplicated between the root and `apps/server` for good
reasons, so the duplication is asserted instead of left to drift.
"""

import configparser
import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

_SERVER_ROOT = ROOT / "apps" / "server"


def _paths(base: Path, values) -> set[Path]:
    """Resolve testpaths to absolute paths relative to their own config."""
    return {(base / value.strip()).resolve() for value in values if value.strip()}


def ruff_check_paths(ci: str) -> list[str]:
    """Directories passed to ``ruff check`` in the CI workflow.

    Strip YAML comments and shell continuations first, so a comment mentioning
    `ruff check`, a reformatted command or a `ruff format --check` invocation
    cannot be mistaken for the lint step.
    """
    commands = "\n".join(re.sub(r"(?<!\S)#.*$", "", line) for line in ci.splitlines())
    commands = re.sub(r"\\\s*\n\s*", " ", commands)
    for line in commands.splitlines():
        tokens = line.split()
        if "ruff" not in tokens:
            continue
        tail = tokens[tokens.index("ruff") + 1 :]
        if "check" not in tail:
            continue
        return [
            token
            for token in tail[tail.index("check") + 1 :]
            if re.fullmatch(r"[A-Za-z0-9_./-]+", token.strip("'\""))
        ]
    raise AssertionError("CI workflow has no `ruff check` command")


def test_pytest_configs_agree_on_mode_and_testpaths():
    root = configparser.ConfigParser()
    root.read(ROOT / "pytest.ini", encoding="utf-8")
    server = tomllib.loads((ROOT / "apps/server/pyproject.toml").read_text(encoding="utf-8"))

    pytest_options = server["tool"]["pytest"]["ini_options"]
    # `pythonpath` is a root-only concern (the container mounts the checkout at
    # /workspace), but both must agree about how tests run and where they live.
    assert pytest_options["asyncio_mode"] == root["pytest"]["asyncio_mode"]
    # Both configs name the same directories, relative to their own root: the
    # root config says `tests apps/server/tests`, the server config says
    # `../../tests` and `tests`. Compare resolved full paths, not basenames —
    # `../wrong/tests` used to look identical to the real directory.
    root_paths = _paths(ROOT, re.split(r"[,\s]+", root["pytest"]["testpaths"]))
    server_paths = _paths(_SERVER_ROOT, pytest_options["testpaths"])
    assert root_paths == server_paths
    assert ROOT / "tests" in root_paths


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
    linted = ruff_check_paths(ci)
    for surface in (
        "apps/server",
        "runners",
        "deploy",
        "scripts",
        "tests",
        "experiments",
        "packages",
    ):
        assert surface in linted, f"{surface} is not linted in CI"


def test_compose_test_database_has_no_committed_password():
    compose = (ROOT / "compose.yaml").read_text(encoding="utf-8")
    # The throwaway test DB uses trust auth on an internal network; a password
    # file in the repository would be a real (if low-value) credential.
    assert "test-db-password" not in compose
    assert not (ROOT / "deploy/docker/test-db-password").exists()
