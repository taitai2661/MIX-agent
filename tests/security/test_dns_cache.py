"""DNS answers are pinned briefly and re-validated on every refresh."""

import socket

import pytest
from mix_agent.tools import network


def _addresses(*ips):
    return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", (ip, 443)) for ip in ips]


@pytest.fixture(autouse=True)
def clean_cache(monkeypatch):
    with network._DNS_LOCK:
        network._DNS_CACHE.clear()
    clock = {"now": 1000.0}
    monkeypatch.setattr(network._time, "monotonic", lambda: clock["now"])
    yield clock
    with network._DNS_LOCK:
        network._DNS_CACHE.clear()


def test_resolution_is_cached_within_ttl(clean_cache, monkeypatch):
    calls = []

    def resolve(host, port, **_kwargs):
        calls.append((host, port))
        return _addresses("93.184.216.34")

    monkeypatch.setattr(network.socket, "getaddrinfo", resolve)
    assert network.public_address_sync("cache-hit.example", 443) == "93.184.216.34"
    assert network.public_address_sync("cache-hit.example", 443) == "93.184.216.34"
    assert calls == [("cache-hit.example", 443)]


def test_refresh_validates_every_address_and_fails_closed(clean_cache, monkeypatch):
    answers = [
        _addresses("93.184.216.34"),
        _addresses("93.184.216.34", "10.0.0.5"),
    ]

    def resolve(*_args, **_kwargs):
        return answers.pop(0)

    monkeypatch.setattr(network.socket, "getaddrinfo", resolve)
    assert network.public_address_sync("refresh.example", 443) == "93.184.216.34"
    clean_cache["now"] += network.DNS_CACHE_TTL_SECONDS + 1
    with pytest.raises(ValueError, match="Private"):
        network.public_address_sync("refresh.example", 443)
    # A failed refresh is not cached: the next one can succeed again.
    answers.append(_addresses("93.184.216.35"))
    assert network.public_address_sync("refresh.example", 443) == "93.184.216.35"
