from deploy.egress.proxy import host_allowed


def test_empty_domain_policy_allows_public_hosts():
    assert host_allowed("example.com", [])


def test_domain_policy_requires_exact_match():
    assert host_allowed("example.com", ["example.com"])
    assert not host_allowed("sub.example.com", ["example.com"])
    assert not host_allowed("other.example.com", ["example.com"])
