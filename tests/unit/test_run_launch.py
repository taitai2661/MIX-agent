"""Regression tests for the run-launch path.

`launch()` creates an asyncio task. It is reached from two very different
places:

* the scheduler and the async routes, which already run on the event loop;
* the *sync* FastAPI routes (`POST /runs/{key}/resume`,
  `POST /runs/{key}/budget-extension`, `POST /approvals/{key}/decision`,
  `POST /scheduled-jobs/{key}/run`), which FastAPI runs in a worker thread
  where no loop is running.

In the second case `asyncio.create_task` raised
`RuntimeError: no running event loop`, so every one of those actions answered
500 — after the database had already been committed, so the run really was
queued and the UI showed a failure for work that was about to start.

These tests pin the dispatch behaviour that fixes it.
"""

from __future__ import annotations

import asyncio
import threading

import pytest

from mix_agent.runs import engine
from mix_agent.runs.engine_support import _next_launch_epoch


@pytest.fixture(autouse=True)
def isolated_engine_state():
    """Snapshot and restore the launch state that lives on the engine module.

    ``TASKS``, ``_PENDING`` and ``_LOOP`` outlive any single test. Clearing or
    rebinding them without restoring left a dead loop (and other tests' runs)
    visible to whatever ran next. The fixture makes the restoration happen even
    when an assertion in the middle of a test fails.
    """
    tasks = dict(engine.TASKS)
    pending = set(engine._PENDING)
    loop = engine._LOOP
    try:
        yield
    finally:
        engine.TASKS.clear()
        engine.TASKS.update(tasks)
        engine._PENDING.clear()
        engine._PENDING.update(pending)
        engine._LOOP = loop


class _FakeSession:
    """A `SessionLocal()` stand-in: the run row does not exist."""

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def get(self, *args, **kwargs):
        return None

    def commit(self):
        return None


def _stub_db(monkeypatch):
    """Replace the DB session so `launch()` needs no database."""
    monkeypatch.setattr(engine, "SessionLocal", _FakeSession)


async def _wait_for_task(run_id):
    """Wait until the driver task for `run_id` is registered."""
    try:
        async with asyncio.timeout(5.0):
            while True:
                if run_id in engine.TASKS:
                    return engine.TASKS[run_id]
                await asyncio.sleep(0.01)
    except TimeoutError:
        return None


async def _cancel_current_tasks():
    """Cancel and drain every task the current scenario started."""
    tasks = [task for task in engine.TASKS.values() if not task.done()]
    for task in tasks:
        task.cancel()
    if tasks:
        await asyncio.gather(*tasks, return_exceptions=True)


def test_launch_from_worker_thread_uses_the_bound_loop(monkeypatch):
    """A launch from a worker thread must run on the bound engine loop."""
    seen: dict[str, object] = {}

    async def fake_drive(run_id):
        seen["loop"] = asyncio.get_running_loop()
        seen["run_id"] = run_id
        # Yield once so the task is really scheduled, not just created.
        await asyncio.sleep(0)

    monkeypatch.setattr(engine, "drive", fake_drive)
    _stub_db(monkeypatch)

    async def scenario():
        engine.bind_loop(asyncio.get_running_loop())
        engine.TASKS.clear()
        engine._PENDING.clear()
        try:
            # This is what a sync FastAPI route does: FastAPI hands the
            # handler to its threadpool, so the call happens off-loop.
            result: dict[str, object] = {}

            def worker():
                engine.launch("run-from-thread")
                result["done"] = True

            thread = threading.Thread(target=worker)
            thread.start()
            # `thread.join` would block the very loop the spawn is waiting
            # for, so it is awaited through the executor.
            await asyncio.to_thread(thread.join, 5)
            assert not thread.is_alive(), "the worker thread did not finish"
            assert result.get("done") is True

            task = await _wait_for_task("run-from-thread")
            assert task is not None, "the drive task was not registered"
            # A real `asyncio.Task` is what makes `cancel()` and the
            # shutdown `gather` in the lifespan work.
            assert isinstance(task, asyncio.Task)
            await task
            assert seen["run_id"] == "run-from-thread"
            assert seen["loop"] is asyncio.get_running_loop()
        finally:
            await _cancel_current_tasks()
            engine.TASKS.clear()
            engine._PENDING.clear()

    asyncio.run(scenario())


def test_launch_does_not_block_the_calling_thread(monkeypatch):
    """The hand-off must return while the loop is still unable to run it.

    The loop can itself be waiting on this thread (`asyncio.to_thread`), so a
    blocking hand-off would deadlock the two against each other. Instead of
    timing the call, this test holds the loop behind an event and waits for the
    worker's own signal: the launch must have returned *before* the gate opens.
    """
    driver_started = asyncio.Event()

    async def fake_drive(run_id):
        driver_started.set()

    monkeypatch.setattr(engine, "drive", fake_drive)
    _stub_db(monkeypatch)

    async def scenario():
        engine.bind_loop(asyncio.get_running_loop())
        engine.TASKS.clear()
        engine._PENDING.clear()
        try:
            gate = asyncio.Event()
            gate_entered = asyncio.Event()

            async def blocker():
                gate_entered.set()
                await gate.wait()

            blocker_task = asyncio.create_task(blocker())
            await gate_entered.wait()

            returned = threading.Event()

            def worker():
                engine.launch("run-blocked")
                returned.set()

            thread = threading.Thread(target=worker)
            thread.start()
            # Awaiting on this loop would need the very loop we are keeping
            # busy, so poll from a plain thread with a generous deadline.
            watcher = threading.Thread(target=lambda: returned.wait(10))
            watcher.start()
            try:
                watcher.join(timeout=12)
                assert returned.is_set(), (
                    "engine.launch() waited for the busy loop; the hand-off must be asynchronous"
                )
                assert not thread.is_alive(), "the launch blocked the calling thread"
                gate.set()
                await blocker_task
                await asyncio.wait_for(driver_started.wait(), 5)
                assert driver_started.is_set(), "the hand-off never started the driver"
            finally:
                gate.set()
                if not blocker_task.done():
                    await blocker_task
                await asyncio.to_thread(thread.join, 5)
        finally:
            await _cancel_current_tasks()
            engine.TASKS.clear()
            engine._PENDING.clear()

    asyncio.run(scenario())


def test_concurrent_launch_of_one_run_starts_one_driver(monkeypatch):
    """Two launches of the same run must not start two drivers."""
    starts: list[str] = []
    first_started = asyncio.Event()
    release = asyncio.Event()

    async def fake_drive(run_id):
        starts.append(run_id)
        first_started.set()
        # Hold the driver so duplicate starts are observable: every launch has
        # been handed off before we look at the count.
        await release.wait()

    monkeypatch.setattr(engine, "drive", fake_drive)
    _stub_db(monkeypatch)

    async def scenario():
        engine.bind_loop(asyncio.get_running_loop())
        engine.TASKS.clear()
        engine._PENDING.clear()
        try:

            def worker():
                engine.launch("run-once")

            threads = [threading.Thread(target=worker) for _ in range(4)]
            for thread in threads:
                thread.start()
            for thread in threads:
                await asyncio.to_thread(thread.join, 5)
                assert not thread.is_alive(), "a launch did not return"

            # All four call_soon_threadsafe hand-offs were queued before this
            # marker, so when it runs the loop has processed every launch.
            flushed = asyncio.Event()
            asyncio.get_running_loop().call_soon(flushed.set)
            await flushed.wait()
            await asyncio.wait_for(first_started.wait(), 5)
            assert starts == ["run-once"], f"the run was started {len(starts)} times"

            release.set()
            task = await _wait_for_task("run-once")
            assert task is not None, "the drive task was not registered"
            await task
        finally:
            release.set()
            await _cancel_current_tasks()
            engine.TASKS.clear()
            engine._PENDING.clear()

    asyncio.run(scenario())


def test_launch_on_the_loop_registers_a_task(monkeypatch):
    """Launching from the loop itself must stay on the same path."""
    seen: dict[str, object] = {}

    async def fake_drive(run_id):
        seen["loop"] = asyncio.get_running_loop()

    monkeypatch.setattr(engine, "drive", fake_drive)
    _stub_db(monkeypatch)

    async def scenario():
        engine.bind_loop(asyncio.get_running_loop())
        engine.TASKS.clear()
        engine._PENDING.clear()
        try:
            engine.launch("run-on-loop")
            task = engine.TASKS.get("run-on-loop")
            assert task is not None
            assert isinstance(task, asyncio.Task)
            await task
            assert seen["loop"] is asyncio.get_running_loop()
        finally:
            await _cancel_current_tasks()
            engine.TASKS.clear()
            engine._PENDING.clear()

    asyncio.run(scenario())


def test_launch_epoch_is_unique_across_threads():
    """The epoch guards against a stale task overwriting a newer run.

    It is allocated from the scheduler's threadpool and from the request
    threadpool, so two concurrent launches must never receive the same value
    or the guard silently stops working.
    """
    epochs: list[int] = []
    lock = threading.Lock()

    def worker():
        for _ in range(200):
            epoch = _next_launch_epoch()
            with lock:
                epochs.append(epoch)

    threads = [threading.Thread(target=worker) for _ in range(8)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=10)

    assert len(epochs) == 8 * 200
    assert len(set(epochs)) == len(epochs), "a launch epoch was handed out twice"
