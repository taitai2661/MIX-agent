"""Installer state and retry behavior without a network download."""
import asyncio

import pytest

from runners.browser_provisioner import main as installer


@pytest.fixture(autouse=True)
def isolated_state(tmp_path, monkeypatch):
    monkeypatch.setattr(installer, "ROOT", tmp_path)
    monkeypatch.setattr(installer, "STATE", tmp_path / "install-state.json")
    monkeypatch.setattr(installer, "TASK", None)


async def test_progress_and_verified_completion(monkeypatch):
    class Output:
        def __init__(self):
            self.chunks = [b"Downloading Chromium 40%\r", b"100%\n", b""]

        async def read(self, _size):
            return self.chunks.pop(0)

    class Process:
        stdout = Output()
        returncode = 0

        async def wait(self):
            return self.returncode

    async def launch(*_args, **_kwargs):
        return Process()

    verified = False

    async def verify():
        nonlocal verified
        assert installer.current()["progress"] == 95
        verified = True

    monkeypatch.setattr(installer.asyncio, "create_subprocess_exec", launch)
    monkeypatch.setattr(installer, "verify_browser", verify)
    first = await installer.start_install()
    second = await installer.start_install()
    assert first["status"] == second["status"] == "installing"
    await installer.TASK
    assert verified
    assert installer.current()["progress"] == 100
    assert installer.current()["status"] == "ready"


async def test_interrupted_state_and_retry(monkeypatch):
    installer.save({"status": "installing", "failure": None, "progress": 50})
    assert installer.current()["failure"] == "install_interrupted"

    async def fail(*_args, **_kwargs):
        raise OSError("network unavailable")

    monkeypatch.setattr(installer.asyncio, "create_subprocess_exec", fail)
    await installer.start_install()
    await installer.TASK
    assert installer.current()["status"] == "failed"
    assert (await installer.start_install())["status"] == "installing"
    await installer.TASK


async def test_ready_install_is_reverified(monkeypatch):
    installer.save({"status": "ready", "failure": None, "progress": 100})
    calls = 0

    async def verify():
        nonlocal calls
        calls += 1

    monkeypatch.setattr(installer, "verify_browser", verify)
    await installer.start_install()
    await installer.TASK
    assert calls == 1
    assert installer.current()["status"] == "ready"


async def test_timeout_kills_install_process(monkeypatch):
    class Output:
        async def read(self, _size):
            await asyncio.sleep(1)

    class Process:
        stdout = Output()
        returncode = None
        killed = False

        def kill(self):
            self.killed = True
            self.returncode = -9

        async def wait(self):
            return self.returncode

    process = Process()

    async def launch(*_args, **_kwargs):
        return process

    monkeypatch.setattr(installer.asyncio, "create_subprocess_exec", launch)
    monkeypatch.setattr(installer, "INSTALL_TIMEOUT", 0.01)
    await installer.start_install()
    await installer.TASK
    assert process.killed
    assert installer.current()["failure"] == "install_timeout"
