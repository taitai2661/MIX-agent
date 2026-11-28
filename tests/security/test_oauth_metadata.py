"""The WWW-Authenticate CIMD hint is untrusted input (finding 8)."""

import pytest
from mix_agent.mcp import oauth


@pytest.fixture
def probe(monkeypatch):
    calls = []

    async def request(_method, _url, **_kwargs):
        return 401, {"www-authenticate": calls.pop(0)}, b""

    async def js(url):
        return {
            "resource": "https://mcp.example.com/mcp",
            "authorization_servers": ["https://id.example.com"],
            "issuer": "https://id.example.com",
        }

    monkeypatch.setattr(oauth, "request_public", request)
    monkeypatch.setattr(oauth, "_json", js)
    return calls


@pytest.mark.parametrize(
    "challenge",
    [
        'Bearer resource_metadata="http://mcp.example.com/.well-known/oauth-protected-resource"',
        'Bearer resource_metadata="https://attacker.example/.well-known/oauth-protected-resource"',
        'Bearer resource_metadata="https://user:pass@mcp.example.com/meta"',
        'Bearer resource_metadata="https://mcp.example.com:8443/meta"',
    ],
)
async def test_unsafe_resource_metadata_url_is_rejected(probe, challenge):
    probe.append(challenge)
    with pytest.raises(ValueError, match="same-origin"):
        await oauth.discover("https://mcp.example.com/mcp")


async def test_same_origin_metadata_url_is_used(probe, monkeypatch):
    captured = []

    async def js(url):
        captured.append(url)
        return {
            "resource": "https://mcp.example.com/mcp",
            "authorization_servers": ["https://id.example.com"],
            "issuer": "https://id.example.com",
        }

    monkeypatch.setattr(oauth, "_json", js)
    probe.append('Bearer resource_metadata="https://mcp.example.com/custom-metadata"')
    result = await oauth.discover("https://mcp.example.com/mcp")
    assert captured[0] == "https://mcp.example.com/custom-metadata"
    assert result["issuer"] == "https://id.example.com"


async def test_resource_identifier_must_match_the_discovered_origin(probe, monkeypatch):
    """A metadata document cannot redefine the resource as another origin."""
    probe.append('Bearer resource_metadata="https://mcp.example.com/meta"')

    async def evil_metadata(_url):
        return {
            "resource": "https://attacker.example/mcp",
            "authorization_servers": ["https://id.example.com"],
        }

    monkeypatch.setattr(oauth, "_json", evil_metadata)
    with pytest.raises(ValueError, match="resource identifier"):
        await oauth.discover("https://mcp.example.com/mcp")


async def test_metadata_fetch_does_not_follow_redirects(monkeypatch):
    """_json uses the pinned, redirect-free request path, not fetch_public."""
    captured = {}

    async def request(method, url, **kwargs):
        captured.update(method=method, url=url, **kwargs)
        return 200, {}, b'{"issuer": "https://id.example.com"}'

    async def forbidden_fetch(*_args, **_kwargs):
        pytest.fail("metadata must not use the redirect-following fetch_public()")

    import mix_agent.tools.network as network

    monkeypatch.setattr(oauth, "request_public", request)
    monkeypatch.setattr(network, "fetch_public", forbidden_fetch)
    value = await oauth._json("https://mcp.example.com/.well-known/oauth-protected-resource")
    assert value == {"issuer": "https://id.example.com"}
    assert captured["method"] == "GET"
    assert captured["url"] == "https://mcp.example.com/.well-known/oauth-protected-resource"


async def test_metadata_fetch_rejects_error_status(monkeypatch):
    async def request(*_args, **_kwargs):
        return 302, {"location": "https://attacker.example/meta"}, b""

    monkeypatch.setattr(oauth, "request_public", request)
    with pytest.raises(ValueError, match="status 302"):
        await oauth._json("https://mcp.example.com/meta")


@pytest.mark.parametrize(
    "endpoint",
    [
        "http://id.example.com/authorize",
        "https://attacker.example/authorize",
        "https://user:pass@id.example.com/authorize",
    ],
)
async def test_server_endpoints_must_stay_on_the_issuer_origin(probe, monkeypatch, endpoint):
    """Endpoint URLs in metadata cannot redirect secrets to another origin."""
    probe.append('Bearer resource_metadata="https://mcp.example.com/meta"')

    async def js(url):
        if url.startswith("https://id.example.com/"):
            return {
                "issuer": "https://id.example.com",
                "authorization_endpoint": endpoint,
                "token_endpoint": "https://id.example.com/token",
            }
        return {
            "resource": "https://mcp.example.com/mcp",
            "authorization_servers": ["https://id.example.com"],
        }

    monkeypatch.setattr(oauth, "_json", js)
    with pytest.raises(ValueError, match="endpoint"):
        await oauth.discover("https://mcp.example.com/mcp")


async def test_same_origin_server_endpoints_are_accepted(probe, monkeypatch):
    probe.append('Bearer resource_metadata="https://mcp.example.com/meta"')

    async def js(url):
        if url.startswith("https://id.example.com/"):
            return {
                "issuer": "https://id.example.com",
                "authorization_endpoint": "https://id.example.com/authorize",
                "token_endpoint": "https://id.example.com/token",
            }
        return {
            "resource": "https://mcp.example.com/mcp",
            "authorization_servers": ["https://id.example.com"],
        }

    monkeypatch.setattr(oauth, "_json", js)
    discovery = await oauth.discover("https://mcp.example.com/mcp")
    assert discovery["metadata"]["token_endpoint"] == "https://id.example.com/token"
