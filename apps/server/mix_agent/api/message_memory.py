"""Best-effort handling of an explicit "remember / forget" request.

Kept separate from the Route module because the Run has already been committed
by the time this runs: a memory failure must be logged and rolled back on its
own, never turn a delivered message into an error.
"""

import logging

from mix_agent.api.message_context import _handle_explicit_memory_requests

LOGGER = logging.getLogger(__name__)


def commit_explicit_memory(db, owner_id, content, temporary_mode):
    try:
        _handle_explicit_memory_requests(db, owner_id, content, temporary_mode)
        db.commit()
    except Exception:  # noqa: BLE001 - the message itself already succeeded
        db.rollback()
        LOGGER.exception("explicit memory request could not be saved")
