"""Structured logging formatter (MIX_LOG_FORMAT=json)."""

import json
import logging

from mix_agent.logging_setup import JsonFormatter


def test_json_formatter_emits_one_object_with_extras():
    record = logging.LogRecord(
        name="mix_agent.http",
        level=logging.INFO,
        pathname=__file__,
        lineno=1,
        msg="request method=%s path=%s",
        args=("GET", "/health"),
        exc_info=None,
    )
    record.request_id = "abc123"

    payload = json.loads(JsonFormatter().format(record))

    assert payload["level"] == "INFO"
    assert payload["logger"] == "mix_agent.http"
    assert payload["message"] == "request method=GET path=/health"
    assert payload["request_id"] == "abc123"
    assert payload["ts"].endswith("Z")


def test_json_formatter_serialises_non_json_extras():
    record = logging.LogRecord(
        name="mix_agent.runs",
        level=logging.WARNING,
        pathname=__file__,
        lineno=1,
        msg="unserialisable",
        args=(),
        exc_info=None,
    )
    record.detail = {1, 2}

    payload = json.loads(JsonFormatter().format(record))

    assert payload["detail"] == "{1, 2}"
