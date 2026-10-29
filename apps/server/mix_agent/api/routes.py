"""HTTP API composition root.

The endpoints live in domain modules (`routes_conversations`, `routes_runs`,
...). This module keeps the historical import surface working:

* ``from mix_agent.api.routes import router`` still returns the generic
  collection router mounted by ``main.py``.
* ``context``, ``own`` and the conversation helpers remain re-exported for
  ``backups.py``, ``helpers/scheduling.py`` and the integration tests that
  reach into them directly.

New code should import from the domain module instead of this aggregator.
"""

from mix_agent.api.common import context, own
from mix_agent.api.routes_collections import router
from mix_agent.api.routes_conversations import effective_tool_ids
from mix_agent.api.routes_messages import (
    _handle_explicit_memory_requests,
    _model_window_info,
)
from mix_agent.storage.cleanup import (
    _delete_message_tail,
    _doomed_messages,
    purge_conversation,
)

__all__ = [
    "_delete_message_tail",
    "_doomed_messages",
    "_handle_explicit_memory_requests",
    "_model_window_info",
    "context",
    "effective_tool_ids",
    "own",
    "purge_conversation",
    "router",
]
