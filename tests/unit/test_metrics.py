from types import SimpleNamespace

from fastapi.routing import APIRoute

from mix_agent import metrics


def test_route_label_uses_the_route_template_not_the_raw_path():
    # Arbitrary unmatched URLs must not become one series each.
    assert metrics.route_label({}) == "/other"
    assert metrics.route_label({"route": SimpleNamespace(path="/api/v1/runs/{key}")}) == "/api/v1/runs/{key}"
    assert metrics.route_label({"route": SimpleNamespace(path="/{path:path}")}) == "/spa"


def test_fastapi_puts_the_template_in_the_request_scope():
    """The middleware relies on ``scope["route"]``, which FastAPI fills in.

    Checking it here (instead of maintaining a custom route class) catches a
    framework upgrade that stops exposing the matched route.
    """
    route = APIRoute("/api/v1/runs/{key}", lambda: None, methods=["GET"])
    match, child_scope = route.matches(
        {
            "type": "http",
            "path": "/api/v1/runs/one",
            "method": "GET",
            "headers": [],
            "query_string": b"",
            "root_path": "",
        }
    )
    assert match.name == "FULL"
    assert metrics.route_label(child_scope) == "/api/v1/runs/{key}"


def test_metrics_keep_the_label_they_are_given():
    # Callers pass `route_label(...)`, so ids never reach the counter and the
    # template must not be truncated to a coarser prefix.
    metrics.reset()
    metrics.observe("GET", "/api/v1/runs/{key}", 200, 0.25)
    metrics.observe("GET", "/api/v1/runs/{key}", 200, 0.75)
    metrics.observe("POST", "/api/v1/runs", 500, 1.0)

    text = metrics.render()

    assert 'path="/api/v1/runs/{key}"' in text
    assert 'path="/api/v1/runs",status="500"' in text
    assert "/api/v1/runs/one" not in text
    assert "mix_http_requests_total" in text
    assert "mix_http_request_duration_seconds_count" in text
    assert "mix_process_uptime_seconds" in text
    metrics.reset()
