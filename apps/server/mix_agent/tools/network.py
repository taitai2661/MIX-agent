import asyncio
import ipaddress
import socket
from urllib.parse import urlsplit
import httpx


async def public_address(host, port):
    addresses = await asyncio.get_running_loop().getaddrinfo(host, port, type=socket.SOCK_STREAM)
    ips = list(dict.fromkeys(a[4][0] for a in addresses))
    if not ips or any(not ipaddress.ip_address(ip).is_global for ip in ips):
        raise ValueError("Private, loopback and metadata addresses are blocked")
    return ips[0]


async def fetch_public_bytes(url, max_bytes=2_000_000):
    """Fetch raw bytes from a public HTTP(S) URL with SSRF protections."""
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
        ip = await public_address(parsed.hostname, port)
        target = httpx.URL(url).copy_with(host=ip)
        async with (
            httpx.AsyncClient(timeout=20, trust_env=False) as client,
            client.stream(
                "GET",
                target,
                headers={"Host": parsed.netloc},
                # httpcore passes this value through to the TLS backend, which
                # expects a hostname string.  Bytes fail there with an
                # AttributeError before the page can be opened.
                extensions={"sni_hostname": parsed.hostname},
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


async def fetch_public(url, max_bytes=2_000_000):
    content, _ = await fetch_public_bytes(url, max_bytes)
    return content.decode("utf-8", errors="replace")


async def request_public(method, url, *, headers=None, data=None, json=None, max_bytes=2_000_000):
    """Issue one pinned public HTTP(S) request without following redirects."""
    parsed = urlsplit(url)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.port not in (None, 443)
    ):
        raise ValueError("Only public HTTPS endpoints are allowed")
    ip = await public_address(parsed.hostname, 443)
    target = httpx.URL(url).copy_with(host=ip)
    request_headers = {"Host": parsed.netloc, **(headers or {})}
    async with (
        httpx.AsyncClient(timeout=20, trust_env=False, follow_redirects=False) as client,
        client.stream(
            method,
            target,
            headers=request_headers,
            data=data,
            json=json,
            extensions={"sni_hostname": parsed.hostname},
        ) as response,
    ):
        content = bytearray()
        async for block in response.aiter_bytes():
            content.extend(block)
            if len(content) > max_bytes:
                raise ValueError("Response exceeds size limit")
            return response.status_code, dict(response.headers), bytes(content)
