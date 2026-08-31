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


async def fetch_public(url, max_bytes=2_000_000):
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
        async with httpx.AsyncClient(timeout=20, trust_env=False) as client:
            async with client.stream(
                "GET",
                target,
                headers={"Host": parsed.netloc},
                extensions={"sni_hostname": parsed.hostname.encode()},
            ) as response:
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
                return bytes(content).decode("utf-8", errors="replace")
    raise ValueError("Too many redirects")
