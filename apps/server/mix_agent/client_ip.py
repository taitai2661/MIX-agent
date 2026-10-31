"""Resolve the client address behind an explicitly trusted reverse proxy.

``request.client.host`` is the TCP peer. Behind a reverse proxy that is the
proxy's address, so using it directly makes every client share one rate-limit
bucket and makes a private-network check (``/metrics``) trust everybody. This
module only honours ``X-Forwarded-For`` when the peer itself is in the trusted
proxy set; otherwise the header is ignored, because any client can send it.

Set ``MIX_TRUSTED_PROXIES`` (comma-separated IPs or CIDRs) to the address the
proxy connects from. ``FORWARDED_ALLOW_IPS`` is accepted as a fallback for
deployments that already configure uvicorn's equivalent setting.
"""

from __future__ import annotations

import ipaddress
import os
from functools import lru_cache

DEFAULT_TRUSTED_PROXIES = "127.0.0.1,::1"


def _env_trusted_proxies() -> str:
    return os.getenv("MIX_TRUSTED_PROXIES") or os.getenv("FORWARDED_ALLOW_IPS") or DEFAULT_TRUSTED_PROXIES


def _parse_ip(value: str):
    """Parse one address, accepting ``[v6]:port`` and ``v4:port`` forms."""
    value = value.strip()
    if not value:
        return None
    if value.startswith("["):
        end = value.find("]")
        if end == -1:
            return None
        value = value[1:end]
    elif value.count(":") == 1 and "." in value:
        value = value.rsplit(":", 1)[0]
    try:
        return ipaddress.ip_address(value)
    except ValueError:
        return None


@lru_cache(maxsize=16)
def _networks(raw: str):
    networks = []
    for item in raw.split(","):
        item = item.strip()
        if not item or item == "*":
            continue
        try:
            networks.append(ipaddress.ip_network(item, strict=False))
        except ValueError:
            # A typo must not make every proxy trusted; skip the entry and
            # keep the rest of the list effective.
            continue
    return tuple(networks)


def _is_trusted(address, networks) -> bool:
    return any(address in network for network in networks)


def resolve_client_ip(peer: str, forwarded_for: str | None) -> str:
    """Return the effective client IP for rate limits and private checks.

    The right-most ``X-Forwarded-For`` entry is the one appended by the proxy
    nearest to us. Walk from there towards the client and stop at the first
    address that is not itself a trusted proxy; that is the only entry an
    untrusted client could not have forged.
    """
    peer_addr = _parse_ip(peer)
    if peer_addr is None:
        return peer or "unknown"
    networks = _networks(_env_trusted_proxies())
    if not _is_trusted(peer_addr, networks) or not forwarded_for:
        return str(peer_addr)

    hops = [address for address in map(_parse_ip, forwarded_for.split(",")) if address is not None]
    if not hops:
        return str(peer_addr)
    for address in reversed(hops):
        if not _is_trusted(address, networks):
            return str(address)
    # Every hop is a trusted proxy (including the client address, which is
    # possible for a loopback health check). The left-most value is still the
    # most specific thing the chain tells us.
    return str(hops[0])


def request_client_ip(request) -> str:
    """Starlette/FastAPI convenience wrapper around :func:`resolve_client_ip`."""
    return resolve_client_ip(
        request.client.host if request.client else "",
        request.headers.get("x-forwarded-for"),
    )
