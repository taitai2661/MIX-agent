"""In-process wakeups for durable work.

The database remains the source of truth.  These signals only avoid repeatedly
asking it whether anything changed while this single-process server is idle.
"""

from __future__ import annotations

import asyncio
import threading
from typing import Any


class Wakeup:
    def __init__(self):
        self.revision = 0
        self.event: asyncio.Event = asyncio.Event()
        # The loop that owns `event`. Commits can run on a worker thread (the
        # message routes offload the transaction), and `Event.set()` from a
        # foreign thread does not reliably wake a loop that is blocked.
        self.loop: asyncio.AbstractEventLoop | None = None
        self._lock = threading.Lock()

    def bind(self):
        self.event = asyncio.Event()
        try:
            self.loop = asyncio.get_running_loop()
        except RuntimeError:
            self.loop = None

    def notify(self):
        with self._lock:
            self.revision += 1
            event = self.event
            loop = self.loop
        if loop is None or not loop.is_running():
            event.set()
            return
        try:
            current: Any = asyncio.get_running_loop()
        except RuntimeError:
            current = None
        if current is loop:
            event.set()
        else:
            loop.call_soon_threadsafe(event.set)

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
