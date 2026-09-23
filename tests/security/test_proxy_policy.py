from deploy.egress.proxy import host_allowed


def test_empty_domain_policy_allows_public_hosts():
    assert host_allowed("example.com", [])


def test_domain_policy_requires_exact_match():
    assert host_allowed("example.com", ["example.com"])
    assert not host_allowed("sub.example.com", ["example.com"])
    assert not host_allowed("other.example.com", ["example.com"])


def test_load_policy_returns_empty_for_absent_file(tmp_path):
    from deploy.egress.proxy import load_policy

    assert load_policy(tmp_path / "absent.json") == {}


def test_load_policy_fails_closed_on_corrupt_file(tmp_path):
    from deploy.egress.proxy import load_policy

    policy = tmp_path / "policy.json"
    policy.write_text("{not valid json")
    assert load_policy(policy) == {"unreadable": True}


def test_load_policy_parses_valid_file(tmp_path):
    from deploy.egress.proxy import load_policy

    policy = tmp_path / "policy.json"
    policy.write_text('{"allowed_domains": ["example.com"]}')
    assert load_policy(policy) == {"allowed_domains": ["example.com"]}


def test_proxy_idle_timeout_default_is_bounded_and_configurable():
    from deploy.egress.proxy import IDLE_TIMEOUT

    assert 0 < IDLE_TIMEOUT <= 300


async def test_public_address_rejects_private_and_mixed_dns(monkeypatch):
    import socket

    from mix_agent.tools.network import public_address

    async def addresses(*args, **kwargs):
        return [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443)),
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("169.254.169.254", 443)),
        ]

    monkeypatch.setattr("asyncio.BaseEventLoop.getaddrinfo", addresses)
    import pytest
    with pytest.raises(ValueError):
        await public_address("example.com", 443)
