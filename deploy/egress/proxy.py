"""Minimal HTTP CONNECT/forward gateway. Exact-host ACL, public-IP validation and DNS pinning.

Policy files fail closed: an unreadable or corrupt policy denies traffic rather
than silently re-enabling unrestricted egress.  The policy `mode` states intent
explicitly (`public_web`, `restricted`, `none`); legacy files without `mode` are
still honoured, but the inference is logged.
"""

import asyncio
import json
import logging
import os
from pathlib import Path
from urllib.parse import urlsplit

from mix_agent.tools.network import public_address

logger = logging.getLogger("egress.proxy")

POLICY = Path(os.getenv("POLICY_FILE", "/policy/policy.json"))
MCP_POLICY = Path(os.getenv("MCP_POLICY_FILE", "/policy/mcp-policy.json"))
IDLE_TIMEOUT = float(os.getenv("IDLE_TIMEOUT", "120"))
HEADER_TIMEOUT = float(os.getenv("HEADER_TIMEOUT", "10"))
CONNECT_TIMEOUT = float(os.getenv("CONNECT_TIMEOUT", "10"))
MAX_HEADER = 16384


def load_policy(path, *, required=True):
    """Load a policy file, failing closed with a sentinel on any error.

    The global policy is required: a missing file is a deployment error, not
    permission to fall back to unrestricted public egress.  The MCP policy is
    optional, because deployments without an MCP manager may legitimately not
    have created it yet; absence then means "no per-source overrides", while
    the global allow-list still applies.
    """
    if not path.exists():
        if required:
            logger.error("Required policy file is missing: %s", path)
            return {"unreadable": True}
        return {}
    try:
        value = json.loads(path.read_text())
        return value if isinstance(value, dict) else {"unreadable": True}
    except Exception:
        logger.exception("Policy file unreadable: %s", path)
        return {"unreadable": True}


POLICY_MODES = ("public_web", "restricted", "none")


def host_allowed(host, allowed_domains):
    """An empty list means unrestricted public egress; otherwise use exact matches."""
    return not allowed_domains or host in allowed_domains


def normalize_domains(values):
    return [value.lower().rstrip(".") for value in values or [] if isinstance(value, str)]


def host_permitted(mode, host, allowed_domains):
    """Whether ``host`` may be reached under a resolved policy mode."""
    if mode == "public_web":
        return True
    return mode == "restricted" and host in allowed_domains


def resolve_policy(policy, source_policy=None):
    """Resolve the effective ``(mode, allowed_domains)`` for one request source.

    ``mode`` is explicit whenever the policy file says so:

    * ``public_web``  — any public host is reachable (still no private IPs).
    * ``restricted``  — only exact hosts in ``allowed_domains``; an empty list
      therefore denies everything.
    * ``none``        — deny all egress.

    Older policy files have no ``mode`` key.  Their implicit behaviour is kept
    (empty list = ``public_web``, non-empty = ``restricted``) so an upgrade
    cannot silently cut off a working deployment, but the inference is logged
    because it is exactly the ambiguity that makes "empty list" unsafe.
    """
    source = source_policy or {}
    raw_mode = source.get("mode") or policy.get("mode")
    domains = source.get("allowed_domains")
    if domains is None:
        domains = policy.get("allowed_domains", [])
    domains = normalize_domains(domains)
    if raw_mode in POLICY_MODES:
        return raw_mode, domains
    inferred = "public_web" if not domains else "restricted"
    logger.warning(
        "egress policy has no `mode`; inferring %r from %d allowed domain(s)",
        inferred,
        len(domains),
    )
    return inferred, domains


async def transfer(reader, writer):
    try:
        while chunk := await asyncio.wait_for(reader.read(65536), IDLE_TIMEOUT):
            writer.write(chunk)
            await writer.drain()
    finally:
        writer.close()


async def handle(reader, writer):
    upstream = None
    source = writer.get_extra_info("peername")
    source_ip = source[0] if source else ""
    target = None
    try:
        header = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), HEADER_TIMEOUT)
        if len(header) > MAX_HEADER:
            raise ValueError()
        first, *lines = header.decode("latin1").split("\r\n")
        method, raw_target, version = first.split(" ")
        target = raw_target
        if method == "CONNECT":
            host, port = raw_target.rsplit(":", 1)
            host, port = host.lower(), int(port)
        else:
            parsed = urlsplit(raw_target)
            if parsed.scheme != "http" or parsed.username or parsed.password:
                raise ValueError()
            host, port = parsed.hostname.lower(), parsed.port or 80
        policy = load_policy(POLICY)
        mcp_policy = load_policy(MCP_POLICY, required=False)
        # Fail closed: never interpret a broken policy file as allow-all.
        if policy.get("unreadable") or mcp_policy.get("unreadable"):
            raise ValueError()
        source_policy = mcp_policy.get("source_policies", {}).get(source_ip)
        mode, allowed_domains = resolve_policy(policy, source_policy)
        allowed = host_permitted(mode, host, allowed_domains)
        if not allowed or port not in (80, 443):
            logger.info(
                "denied %s -> %s:%d (mode=%s, domains=%s)", source_ip, host, port, mode, allowed_domains
            )
            raise ValueError()
        logger.info("proxy %s %s -> %s:%d", source_ip, method, host, port)
        ip = await public_address(host, port)
        remote, upstream = await asyncio.wait_for(asyncio.open_connection(ip, port), CONNECT_TIMEOUT)
        if method == "CONNECT":
            writer.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        else:
            path = parsed.path or "/"
            if parsed.query:
                path += "?" + parsed.query
            safe = [
                line
                for line in lines
                if line
                and line.split(":", 1)[0].lower()
                not in ("host", "proxy-authorization", "proxy-connection", "connection")
            ]
            upstream.write(
                (
                    f"{method} {path} {version}\r\nHost: {host}\r\nConnection: close\r\n"
                    + "\r\n".join(safe)
                    + "\r\n\r\n"
                ).encode("latin1")
            )
        await writer.drain()
        tasks = [
            asyncio.create_task(transfer(reader, upstream)),
            asyncio.create_task(transfer(remote, writer)),
        ]
        _, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        for task in pending:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
    except Exception as exc:  # noqa: BLE001 - transport boundary; respond and log
        logger.info("request failed from %s to %s: %s", source_ip, target, type(exc).__name__)
        if not writer.is_closing():
            writer.write(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    finally:
        writer.close()
        if upstream:
            upstream.close()


async def main():
    logging.basicConfig(
        level=os.getenv("LOG_LEVEL", "INFO"), format="%(asctime)s %(name)s %(levelname)s %(message)s"
    )
    server = await asyncio.start_server(
        handle,
        "0.0.0.0",  # noqa: S104 - the proxy must be reachable from sibling containers
        3128,
        limit=MAX_HEADER,
    )
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
