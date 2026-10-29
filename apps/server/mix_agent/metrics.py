"""Dependency-free request metrics for the Docker edition.

Prometheus is not a dependency and a single-process deployment does not need a
full tracing stack, but "how many 5xx did we return in the last hour" should
not require grepping logs.  This keeps counters in memory and renders the
Prometheus text format at ``/metrics``.

Labels come from the matched route template (``/api/v1/runs/{key}``) recorded
by FastAPI in ``scope["route"]``, so individual ids and arbitrary unmatched
URLs cannot grow the label set.
"""

from __future__ import annotations

import threading
import time
from collections import Counter
from collections.abc import Mapping
from typing import Any

_LOCK = threading.Lock()
_REQUESTS: Counter[tuple[str, str, int]] = Counter()
_DURATION_SUM: dict[tuple[str, str], float] = {}
_DURATION_COUNT: Counter[tuple[str, str]] = Counter()
_STARTED = time.time()


def route_label(scope: Mapping[str, Any]) -> str:
    """Label a request by its *route template*, never by the raw URL.

    Using ``request.url.path`` let any anonymous caller create an unbounded
    number of counter series just by requesting random paths (``/api/v1/x1``,
    ``/api/v1/x2`` ...).  FastAPI writes the matched route into
    ``scope["route"]``, so the template (``/api/v1/runs/{key}``) is both
    bounded and useful.  Unmatched requests are collapsed into one bucket.
    """
    route = scope.get("route")
    template = getattr(route, "path", "") or ""
    # The SPA catch-all is ``/{path:path}``; it is a single bucket, not a place
    # to keep the literal template.
    if template.startswith("/{"):
        return "/spa"
    return template or "/other"


def observe(method: str, path: str, status: int, seconds: float) -> None:
    """Record one request. ``path`` must already be bounded (a route
    template, ``/spa``, ``/other`` or ``/middleware``); it is used as-is so
    the emitted label is the endpoint the caller intended, not a truncated
    prefix."""
    key = (method, path)
    with _LOCK:
        _REQUESTS[(method, path, status)] += 1
        _DURATION_SUM[key] = _DURATION_SUM.get(key, 0.0) + seconds
        _DURATION_COUNT[key] += 1


def reset() -> None:
    with _LOCK:
        _REQUESTS.clear()
        _DURATION_SUM.clear()
        _DURATION_COUNT.clear()


def render() -> str:
    lines = [
        "# HELP mix_http_requests_total HTTP requests by method, path and status.",
        "# TYPE mix_http_requests_total counter",
    ]
    with _LOCK:
        started = _STARTED
        requests = list(_REQUESTS.items())
        duration_sum = list(_DURATION_SUM.items())
        duration_count = list(_DURATION_COUNT.items())
    for (method, path, status), count in sorted(requests):
        lines.append(f'mix_http_requests_total{{method="{method}",path="{path}",status="{status}"}} {count}')
    lines.extend(
        [
            "# HELP mix_http_request_duration_seconds Request duration in seconds.",
            "# TYPE mix_http_request_duration_seconds summary",
        ]
    )
    for (method, path), total in sorted(duration_sum):
        lines.append(f'mix_http_request_duration_seconds_sum{{method="{method}",path="{path}"}} {total:.6f}')
    for (method, path), count in sorted(duration_count):
        lines.append(f'mix_http_request_duration_seconds_count{{method="{method}",path="{path}"}} {count}')
    lines.extend(
        [
            "# HELP mix_process_uptime_seconds Process uptime.",
            "# TYPE mix_process_uptime_seconds gauge",
            f"mix_process_uptime_seconds {max(0.0, time.time() - started):.3f}",
        ]
    )
    return "\n".join(lines) + "\n"
