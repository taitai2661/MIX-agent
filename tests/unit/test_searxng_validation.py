import pytest

from mix_agent.api.helpers.searxng import validate_searxng_url


def test_searxng_rejects_private_and_loopback_literals():
    for url in ("https://127.0.0.1", "https://10.0.0.5", "https://169.254.169.254"):
        with pytest.raises(ValueError):
            validate_searxng_url(url)


def test_searxng_rejects_non_https_and_custom_ports():
    for url in ("http://search.example.com", "https://search.example.com:8443"):
        with pytest.raises(ValueError):
            validate_searxng_url(url)


def test_searxng_accepts_a_public_literal_and_normalizes_slash():
    assert validate_searxng_url("https://93.184.216.34/") == "https://93.184.216.34"
