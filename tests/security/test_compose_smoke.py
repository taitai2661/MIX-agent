"""Optional real-Compose smoke coverage.

Skipped on hosts without a Docker daemon; the Docker-free equivalents live in
``test_runner_hardening.py`` and exercise the same manager install path against
an in-process fake Docker API.
"""

import json
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
DOCKER = shutil.which("docker")


@pytest.mark.skipif(DOCKER is None, reason="docker CLI is not installed")
def test_compose_profile_installs_a_pinned_npm_mcp():
    daemon = subprocess.run(  # noqa: S603
        [DOCKER, "info"], capture_output=True, text=True, check=False
    )
    if daemon.returncode != 0:
        pytest.skip("docker daemon is unavailable")
    result = subprocess.run(  # noqa: S603
        [
            DOCKER,
            "compose",
            "--profile",
            "test",
            "run",
            "--rm",
            "--no-deps",
            "test",
            "python",
            "-m",
            "pytest",
            "-q",
            "tests/security/test_runner_hardening.py::test_install_registers_a_pinned_npm_package",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr


@pytest.mark.skipif(DOCKER is None, reason="docker CLI is not installed")
def test_compose_renders_isolated_docker_proxy_network():
    """Real Compose rendering must preserve the docker-proxy isolation."""
    daemon = subprocess.run(  # noqa: S603
        [DOCKER, "info"], capture_output=True, text=True, check=False
    )
    if daemon.returncode != 0:
        pytest.skip("docker daemon is unavailable")
    result = subprocess.run(  # noqa: S603
        [DOCKER, "compose", "config", "--format", "json"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        pytest.skip(f"compose config unavailable: {result.stderr.strip()}")
    services = json.loads(result.stdout)["services"]
    proxy_networks = set(services["docker-socket-proxy"]["networks"])
    assert proxy_networks == {"docker-proxy"}
    assert "docker-proxy" in services["mcp-manager"]["networks"]
    for unrelated in ("mcp-runner", "egress-proxy", "app"):
        assert "docker-proxy" not in services[unrelated]["networks"], unrelated
