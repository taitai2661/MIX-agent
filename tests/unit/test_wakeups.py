import asyncio
import threading

from mix_agent.wakeups import Wakeup


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
