from deploy.egress.proxy import host_allowed, host_permitted, resolve_policy


def test_empty_domain_policy_allows_public_hosts():
    assert host_allowed("example.com", [])


def test_host_permitted_follows_the_resolved_mode():
    assert host_permitted("public_web", "anything.example", [])
    assert host_permitted("restricted", "example.com", ["example.com"])
    assert not host_permitted("restricted", "sub.example.com", ["example.com"])
    # The important case: a restricted policy with an empty list must deny,
    # not fall back to "unrestricted public egress".
    assert not host_permitted("restricted", "example.com", [])
    assert not host_permitted("none", "example.com", ["example.com"])


def test_policy_without_mode_keeps_legacy_behaviour():
    assert resolve_policy({"allowed_domains": []}) == ("public_web", [])
    assert resolve_policy({"allowed_domains": ["example.com"]}) == ("restricted", ["example.com"])


def test_explicit_restricted_mode_denies_empty_allow_list():
    assert resolve_policy({"mode": "restricted", "allowed_domains": []}) == ("restricted", [])


def test_explicit_none_mode_denies_everything():
    assert resolve_policy({"mode": "none", "allowed_domains": ["example.com"]})[0] == "none"


def test_source_policy_overrides_global_policy():
    policy = {"mode": "restricted", "allowed_domains": ["example.com"]}
    source = {"mode": "none", "allowed_domains": []}
    assert resolve_policy(policy, source) == ("none", [])
    # A source override without a mode inherits the global explicit mode.
    assert resolve_policy(policy, {"allowed_domains": ["cdn.example.com"]}) == (
        "restricted",
        ["cdn.example.com"],
    )


def test_domains_are_normalised():
    assert resolve_policy({"mode": "restricted", "allowed_domains": ["Example.COM."]})[1] == ["example.com"]


def test_domain_policy_requires_exact_match():
    assert host_allowed("example.com", ["example.com"])
    assert not host_allowed("sub.example.com", ["example.com"])
    assert not host_allowed("other.example.com", ["example.com"])


def test_load_policy_fails_closed_for_absent_required_file(tmp_path):
    from deploy.egress.proxy import load_policy

    assert load_policy(tmp_path / "absent.json") == {"unreadable": True}


def test_load_policy_allows_absent_optional_file(tmp_path):
    from deploy.egress.proxy import load_policy

    assert load_policy(tmp_path / "absent-mcp.json", required=False) == {}


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
