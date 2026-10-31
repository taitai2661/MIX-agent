from mix_agent import client_ip


def test_forwarded_for_is_ignored_from_an_untrusted_peer(monkeypatch):
    monkeypatch.delenv("MIX_TRUSTED_PROXIES", raising=False)
    monkeypatch.delenv("FORWARDED_ALLOW_IPS", raising=False)
    assert client_ip.resolve_client_ip("198.51.100.5", "203.0.113.9") == "198.51.100.5"


def test_trusted_proxy_resolves_the_nearest_untrusted_hop(monkeypatch):
    monkeypatch.setenv("MIX_TRUSTED_PROXIES", "127.0.0.1,10.0.0.0/8")
    assert client_ip.resolve_client_ip("127.0.0.1", "203.0.113.9, 10.0.0.2") == "203.0.113.9"


def test_a_fully_trusted_chain_falls_back_to_its_leftmost_address(monkeypatch):
    monkeypatch.setenv("MIX_TRUSTED_PROXIES", "127.0.0.1,10.0.0.0/8")
    assert client_ip.resolve_client_ip("127.0.0.1", "10.1.1.1, 10.0.0.2") == "10.1.1.1"


def test_ipv6_and_port_forms_are_accepted(monkeypatch):
    monkeypatch.setenv("MIX_TRUSTED_PROXIES", "::1")
    assert client_ip.resolve_client_ip("[::1]:5000", "[2001:db8::1]:443") == "2001:db8::1"


def test_invalid_forwarded_entries_are_ignored(monkeypatch):
    monkeypatch.setenv("MIX_TRUSTED_PROXIES", "127.0.0.1")
    assert client_ip.resolve_client_ip("127.0.0.1", "unknown, 203.0.113.9") == "203.0.113.9"
