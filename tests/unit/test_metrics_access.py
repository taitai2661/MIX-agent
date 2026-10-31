"""`/metrics` is scrapeable locally, but not from an arbitrary client.

The endpoint has no authentication by default; the guard is that only
loopback/private clients may read it unless `METRICS_TOKEN` is set. `TestClient`
reports the peer as `testclient`, which is deliberately neither, so it stands in
for a public client here.
"""

from fastapi.testclient import TestClient

from mix_agent.main import app


def test_untrusted_client_gets_not_found_without_token(monkeypatch):
    monkeypatch.delenv("METRICS_TOKEN", raising=False)
    monkeypatch.delenv("MIX_METRICS_PUBLIC", raising=False)
    response = TestClient(app).get("/metrics")
    assert response.status_code == 404


def test_public_opt_in_restores_unauthenticated_access(monkeypatch):
    monkeypatch.delenv("METRICS_TOKEN", raising=False)
    monkeypatch.setenv("MIX_METRICS_PUBLIC", "1")
    response = TestClient(app).get("/metrics")
    assert response.status_code == 200
    assert "mix_process_uptime_seconds" in response.text


def test_token_is_required_and_enforced_when_configured(monkeypatch):
    monkeypatch.setenv("METRICS_TOKEN", "scrape-token")
    client = TestClient(app)
    assert client.get("/metrics").status_code == 401
    assert client.get("/metrics", headers={"Authorization": "Bearer wrong"}).status_code == 401
    assert client.get("/metrics", headers={"Authorization": "Bearer scrape-token"}).status_code == 200
