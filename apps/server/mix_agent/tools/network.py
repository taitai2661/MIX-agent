"""Public egress helpers shared by the app, the MCP runner and the proxy.

Trust boundary: this module reads exactly one proxy setting, the explicit
``MIX_EGRESS_PROXY``. Generic ``HTTPS_PROXY``/``http_proxy`` variables are
*not* consulted here: a deployment that wants the shared egress proxy must opt
in (the internal-only MCP runner maps its Compose-provided ``HTTPS_PROXY`` into
``MIX_EGRESS_PROXY`` at startup). The proxy container remains the authoritative
policy enforcer. When a local policy file is available the same
``mode``/``allowed_domains`` rules are also checked here (defence in depth, and
it lets app-originated requests honour ``restricted``/``none`` even before they
reach the proxy).
"""

import asyncio
import ipaddress
import json
import os
import socket
import threading
import time as _time
from pathlib import Path
from urllib.parse import urlsplit

import httpx

# A DNS answer is pinned for a short window; every refresh re-validates *all*
# resolved addresses before any of them may be used. Failures are never cached
# so a transient resolver error cannot keep traffic blocked after it clears.
DNS_CACHE_TTL_SECONDS = float(os.getenv("MIX_DNS_CACHE_TTL", "30"))
_DNS_CACHE: dict[tuple[str, int], tuple[float, str]] = {}
_DNS_LOCK = threading.Lock()

LABEL_CHARS = frozenset("abcdefghijklmnopqrstuvwxyz0123456789-")


def validate_domain(value) -> str:
    """Normalise a DNS name for an allow-list, rejecting malformed entries.

    An allow-list names hosts, not IP literals: a numeric entry such as
    ``1.2.3.4`` would silently bypass the proxy's hostname ACL, so it is
    rejected here along with empty labels and labels starting/ending in '-'.
    """
    host = str(value).lower().rstrip(".")
    if not host or len(host) > 253:
        raise ValueError("Invalid allowed domain")
    # Keep the historical requirement that an allow-list entry names a real
    # DNS name, not a bare internal host like `localhost` or `metadata`.
    if "." not in host:
        raise ValueError("Invalid allowed domain")
    try:
        ipaddress.ip_address(host)
    except ValueError:
        pass
    else:
        raise ValueError("Invalid allowed domain")
    for label in host.split("."):
        if not 1 <= len(label) <= 63 or label[0] == "-" or label[-1] == "-" or not set(label) <= LABEL_CHARS:
            raise ValueError("Invalid allowed domain")
    return host


def _domain_set(values) -> set[str]:
    return {validate_domain(value) for value in values or []}


def _local_policy():
    """Return ``(mode, domains)`` when a readable local policy exists.

    ``None`` means "no local policy file", which is the normal case for the
    internal-only MCP runner where the (separate) proxy container owns policy.
    A corrupt or unknown-mode file instead fails closed with ``none``.
    """
    path = Path(
        os.getenv(
            "MIX_EGRESS_POLICY_FILE",
            str(Path(os.getenv("MIX_DATA", "/data")) / "egress" / "policy.json"),
        )
    )
    try:
        value = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    if not isinstance(value, dict) or value.get("unreadable"):
        return "none", set()
    mode = value.get("mode")
    if mode not in ("public_web", "restricted", "none"):
        return "none", set()
    try:
        domains = _domain_set(value.get("allowed_domains", []))
    except ValueError:
        return "none", set()
    return mode, domains


def _enforce_domain_policy(host: str, allowed_domains) -> None:
    """Apply the explicit allow-list and the local global policy, if any."""
    host = host.lower().rstrip(".")
    if allowed_domains and host not in _domain_set(allowed_domains):
        raise ValueError("Domain is not in the allowed list")
    policy = _local_policy()
    if policy is None:
        return
    mode, domains = policy
    if mode == "none":
        raise ValueError("Egress is disabled by policy")
    if mode == "restricted" and host not in domains:
        raise ValueError("Domain is not in the allowed list")


def _egress_proxy_url() -> str:
    """The explicitly configured egress proxy, if any.

    Only ``MIX_EGRESS_PROXY`` is read on purpose; generic proxy environment
    variables must not silently turn app-originated traffic into proxied
    traffic (they may point at a proxy that does not enforce policy).
    """
    return os.getenv("MIX_EGRESS_PROXY", "").strip()


def _global_ips(addresses) -> str:
    ips = list(dict.fromkeys(address[4][0] for address in addresses))
    if not ips or any(not ipaddress.ip_address(ip).is_global for ip in ips):
        raise ValueError("Private, loopback and metadata addresses are blocked")
    return ips[0]


def _cached_address(key: tuple[str, int]) -> str | None:
    with _DNS_LOCK:
        entry = _DNS_CACHE.get(key)
    if not entry:
        return None
    stamped, ip = entry
    if _time.monotonic() - stamped >= DNS_CACHE_TTL_SECONDS:
        with _DNS_LOCK:
            _DNS_CACHE.pop(key, None)
        return None
    # A cached answer is re-checked too: cheap, and it keeps fail-closed
    # behaviour even if an entry was populated by an older code path.
    if not ipaddress.ip_address(ip).is_global:
        raise ValueError("Private, loopback and metadata addresses are blocked")
    return ip


def _remember_address(key: tuple[str, int], addresses) -> str:
    ip = _global_ips(addresses)
    with _DNS_LOCK:
        # Bounded cache: drop fully expired entries before adding a new one.
        now = _time.monotonic()
        for old_key, (stamped, _) in list(_DNS_CACHE.items()):
            if now - stamped >= DNS_CACHE_TTL_SECONDS:
                _DNS_CACHE.pop(old_key, None)
        _DNS_CACHE[key] = (now, ip)
    return ip


async def public_address(host, port):
    """Resolve once per TTL, validating every address on each refresh."""
    key = (str(host).lower().rstrip("."), int(port))
    cached = _cached_address(key)
    if cached:
        return cached
    addresses = await asyncio.get_running_loop().getaddrinfo(host, port, type=socket.SOCK_STREAM)
    return _remember_address(key, addresses)


def public_address_sync(host, port):
    """Synchronous variant for sync FastAPI routes (run in a threadpool)."""
    key = (str(host).lower().rstrip("."), int(port))
    cached = _cached_address(key)
    if cached:
        return cached
    addresses = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    return _remember_address(key, addresses)


async def fetch_public_bytes(url, max_bytes=2_000_000, *, allowed_domains=None):
    """Fetch raw bytes from a public HTTP(S) URL with SSRF protections.

    Like ``request_public``, an explicitly configured egress proxy keeps the
    real hostname in the URL (the proxy pins DNS); without one the legacy
    IP-pinned path with an explicit SNI hostname is used.
    """
    proxy = _egress_proxy_url()
    for _ in range(5):
        parsed = urlsplit(url)
        if (
            parsed.scheme not in ("http", "https")
            or not parsed.hostname
            or parsed.username
            or parsed.password
        ):
            raise ValueError("Only public HTTP(S) URLs are allowed")
        port = parsed.port or (443 if parsed.scheme == "https" else 80)
        if port not in (80, 443):
            raise ValueError("Only ports 80 and 443 are allowed")
        _enforce_domain_policy(parsed.hostname, allowed_domains)
        ip = await public_address(parsed.hostname, port)
        request_headers = {}
        extensions = {}
        if proxy:
            target = url
        else:
            target = httpx.URL(url).copy_with(host=ip)
            request_headers["Host"] = parsed.netloc
            # httpcore passes this value through to the TLS backend, which
            # expects a hostname string. Bytes fail there with an
            # AttributeError before the page can be opened.
            extensions = {"sni_hostname": parsed.hostname}
        async with (
            httpx.AsyncClient(timeout=20, trust_env=False, proxy=proxy or None) as client,
            client.stream(
                "GET",
                target,
                headers=request_headers,
                extensions=extensions,
            ) as response,
        ):
            if response.is_redirect:
                from urllib.parse import urljoin

                url = urljoin(url, response.headers["location"])
                continue
            response.raise_for_status()
            content = bytearray()
            async for block in response.aiter_bytes():
                content.extend(block)
                if len(content) > max_bytes:
                    raise ValueError("Page exceeds size limit")
            return bytes(content), _content_type(response)
    raise ValueError("Too many redirects")


def _content_type(response):
    headers = getattr(response, "headers", None)
    get = getattr(headers, "get", None)
    return str(get("content-type", "") if callable(get) else "")


async def fetch_public(url, max_bytes=2_000_000, *, allowed_domains=None):
    content, _ = await fetch_public_bytes(url, max_bytes, allowed_domains=allowed_domains)
    return content.decode("utf-8", errors="replace")


async def request_public(
    method,
    url,
    *,
    headers=None,
    data=None,
    json=None,
    max_bytes=2_000_000,
    timeout=20,
    allowed_domains=None,
):
    """Issue one pinned public HTTP(S) request without following redirects.

    With an egress proxy configured the URL keeps its hostname (the proxy does
    the DNS pinning); otherwise the legacy IP-pinned path with an explicit SNI
    hostname is used. Both paths enforce the same allow-list/policy first.
    """
    parsed = urlsplit(url)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.port not in (None, 443)
    ):
        raise ValueError("Only public HTTPS endpoints are allowed")
    host = parsed.hostname.lower().rstrip(".")
    _enforce_domain_policy(host, allowed_domains)
    ip = await public_address(host, 443)
    proxy = _egress_proxy_url()
    request_headers = {key: value for key, value in (headers or {}).items() if key.lower() != "host"}
    extensions = {}
    if proxy:
        target = url
    else:
        request_headers["Host"] = parsed.netloc
        target = httpx.URL(url).copy_with(host=ip)
        extensions = {"sni_hostname": host}
    async with (
        httpx.AsyncClient(
            timeout=timeout, trust_env=False, follow_redirects=False, proxy=proxy or None
        ) as client,
        client.stream(
            method,
            target,
            headers=request_headers,
            data=data,
            json=json,
            extensions=extensions,
        ) as response,
    ):
        content = bytearray()
        async for block in response.aiter_bytes():
            content.extend(block)
            if len(content) > max_bytes:
                raise ValueError("Response exceeds size limit")
        return response.status_code, dict(response.headers), bytes(content)
