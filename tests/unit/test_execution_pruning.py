"""Terminal process entries must not accumulate forever (finding 11)."""

import pytest
from runners.execution import main as execution


@pytest.fixture(autouse=True)
def isolated_processes(monkeypatch):
    monkeypatch.setattr(execution, "PROCESSES", {})
    monkeypatch.setattr(execution, "PROCESS_HISTORY_LIMIT", 3)
    monkeypatch.setattr(execution, "PROCESS_TTL_SECONDS", 10)
    monkeypatch.setattr(execution, "PAGES", {})


def _entry(exit_code, age=0, now=1000.0):
    return {
        "process": None,
        "output": "",
        "exit_code": exit_code,
        "finished_at": now - age,
    }


def test_aged_out_terminal_entries_are_removed(monkeypatch):
    clock = {"now": 1000.0}
    monkeypatch.setattr(execution._time, "monotonic", lambda: clock["now"])
    execution.PROCESSES.update(
        {
            "old": _entry(0, age=20),
            "fresh": _entry(0, age=1),
            "running": _entry(None),
        }
    )
    execution.prune_processes()
    assert "old" not in execution.PROCESSES
    assert "fresh" in execution.PROCESSES
    assert "running" in execution.PROCESSES


def test_history_is_bounded_and_never_drops_running_work(monkeypatch):
    clock = {"now": 1000.0}
    monkeypatch.setattr(execution._time, "monotonic", lambda: clock["now"])
    for index in range(10):
        execution.PROCESSES[f"done-{index}"] = _entry(0)
    execution.PROCESSES["running"] = _entry(None)
    execution.prune_processes()
    assert "running" in execution.PROCESSES
    assert len(execution.PROCESSES) <= execution.PROCESS_HISTORY_LIMIT


class _FakeContext:
    def __init__(self):
        self.closed = False

    async def close(self):
        self.closed = True


@pytest.mark.asyncio
async def test_idle_browser_pages_are_closed(monkeypatch):
    monkeypatch.setattr(execution, "PAGE_IDLE_TIMEOUT", 100)
    idle, fresh = _FakeContext(), _FakeContext()
    execution.PAGES.update(
        {
            "idle": {"context": idle, "last_used": 900.0},
            "fresh": {"context": fresh, "last_used": 990.0},
        }
    )
    await execution.prune_idle_pages(now=1000.0)
    assert "idle" not in execution.PAGES
    assert idle.closed
    assert "fresh" in execution.PAGES
    assert not fresh.closed


def test_browser_config_treats_explicit_none_as_unset():
    config = execution.browser_config(
        {
            "browser_locale": None,
            "browser_user_agent": None,
            "browser_timeout_ms": None,
            "browser_viewport_width": None,
        }
    )
    assert config["locale"] == "ja-JP"
    assert config["user_agent"] == ""
    assert config["timeout"] == 15000
    assert config["viewport"]["width"] == 1280
