import asyncio
import gc
import threading

from mix_agent.wakeups import RunWakeups, Wakeup


def test_notify_from_a_worker_thread_wakes_the_loop_waiter():
    """Commits can run on the threadpool; `Event.set` must be marshalled."""

    async def scenario():
        wakeup = Wakeup()
        wakeup.bind()
        seen = wakeup.revision

        worker = threading.Thread(target=wakeup.notify)
        worker.start()
        worker.join(timeout=1)

        assert not worker.is_alive()
        assert await wakeup.wait(seen, timeout=1) > seen

    asyncio.run(scenario())


def test_notify_on_the_owning_loop_still_wakes():
    async def scenario():
        wakeup = Wakeup()
        wakeup.bind()
        seen = wakeup.revision
        wakeup.notify()
        assert await wakeup.wait(seen, timeout=1) > seen

    asyncio.run(scenario())


def test_a_run_wakeup_is_released_when_no_stream_holds_it():
    runs = RunWakeups()
    wakeup = runs.for_run("run-1")
    assert runs.for_run("run-1") is wakeup
    del wakeup
    gc.collect()
    # WeakValueDictionary still exposes a ``len``; the entry must be gone
    # rather than retained for every Run the server has ever streamed.
    assert len(runs._by_run) == 0
