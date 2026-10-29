"""Optional structured logging for the Docker edition.

The default stays the human-readable uvicorn format so local development and
existing log collectors keep working. Setting ``MIX_LOG_FORMAT=json`` switches
every root/uvicorn handler to one JSON object per line, which is what a
self-hosted deployment needs to correlate a request with the run it started:

    {"ts":"2026-10-29T12:34:56.789Z","level":"INFO","logger":"mix_agent.http",
     "message":"request ...","request_id":"..."}

Extra keys are copied from ``LogRecord`` attributes (``request_id``,
``run_id``, ...) so call sites keep using normal ``%``-style logging.
"""

import json
import logging
import os
import sys
from datetime import UTC, datetime

# Attributes every LogRecord carries; anything else was attached by the
# caller and belongs in the JSON object.
_RESERVED = {
    "args",
    "asctime",
    "created",
    "exc_info",
    "exc_text",
    "filename",
    "funcName",
    "levelname",
    "levelno",
    "lineno",
    "module",
    "msecs",
    "message",
    "msg",
    "name",
    "pathname",
    "process",
    "processName",
    "relativeCreated",
    "stack_info",
    "thread",
    "threadName",
    "taskName",
}


class JsonFormatter(logging.Formatter):
    """Render one record as a single-line JSON object."""

    def format(self, record: logging.LogRecord) -> str:
        payload = {
            "ts": datetime.fromtimestamp(record.created, tz=UTC)
            .isoformat(timespec="milliseconds")
            .replace("+00:00", "Z"),
            "level": record.levelname,
            "logger": record.name,
            "message": record.getMessage(),
        }
        for key, value in record.__dict__.items():
            if key in _RESERVED or key.startswith("_"):
                continue
            try:
                json.dumps(value)
            except (TypeError, ValueError):
                value = repr(value)
            payload[key] = value
        if record.exc_info:
            payload["exception"] = self.formatException(record.exc_info)
        return json.dumps(payload, ensure_ascii=False)


def configure_logging() -> None:
    """Apply ``MIX_LOG_FORMAT`` to the handlers already installed by uvicorn.

    Called at import time so it runs after uvicorn's own ``dictConfig``.  It
    never adds a second handler: it only swaps the formatter on handlers that
    already exist, plus the root logger for records emitted before uvicorn
    configures itself.
    """
    if os.getenv("MIX_LOG_FORMAT", "text").strip().lower() != "json":
        return
    formatter = JsonFormatter()
    loggers = [
        logging.getLogger(),
        *(logging.getLogger(name) for name in ("uvicorn", "uvicorn.error", "uvicorn.access")),
    ]
    handlers: list[logging.Handler] = []
    for logger in loggers:
        for handler in logger.handlers:
            if handler not in handlers:
                handlers.append(handler)
    if not handlers:
        handler = logging.StreamHandler(sys.stdout)
        logging.getLogger().addHandler(handler)
        handlers.append(handler)
    for handler in handlers:
        handler.setFormatter(formatter)
