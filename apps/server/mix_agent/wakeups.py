"""In-process wakeups for durable work.

The database remains the source of truth.  These signals only avoid repeatedly
asking it whether anything changed while this single-process server is idle.
"""

from __future__ import annotations

import asyncio


class Wakeup:
    def __init__(self):
        self.revision = 0
        self.event: asyncio.Event | None = None

    def bind(self):
        self.event = asyncio.Event()

    def notify(self):
        self.revision += 1
        if self.event:
            self.event.set()

    async def wait(self, seen: int, timeout: float | None = None) -> int:
        if self.revision != seen:
            return self.revision
        if not self.event:
            self.bind()
        try:
            await asyncio.wait_for(self.event.wait(), timeout)
        except TimeoutError:
            pass
        self.event.clear()
        return self.revision


class RunWakeups:
    def __init__(self):
        self._by_run: dict[str, Wakeup] = {}

    def for_run(self, run_id: str) -> Wakeup:
        return self._by_run.setdefault(run_id, Wakeup())

    def bind(self):
        # Lifespan restarts (including tests) use a fresh event loop.
        self._by_run.clear()

    def notify(self, run_id: str):
        self.for_run(run_id).notify()


run_events = RunWakeups()
scheduler = Wakeup()
memory_jobs = Wakeup()


def mark(session, channel: str, run_id: str | None = None):
    """Defer a notification until the enclosing transaction commits."""
    pending = session.info.setdefault("mix_wakeups", {"runs": set(), "scheduler": False, "memory": False})
    if run_id:
        pending["runs"].add(run_id)
    if channel == "scheduler":
        pending["scheduler"] = True
    elif channel == "memory":
        pending["memory"] = True


def committed(session):
    pending = session.info.pop("mix_wakeups", None)
    if not pending:
        return
    for run_id in pending["runs"]:
        run_events.notify(run_id)
    if pending["scheduler"]:
        scheduler.notify()
    if pending["memory"]:
        memory_jobs.notify()


def rolled_back(session):
    session.info.pop("mix_wakeups", None)
