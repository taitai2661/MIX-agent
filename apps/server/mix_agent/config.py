import os
import socket
from ipaddress import ip_address
from pathlib import Path
from urllib.parse import urlsplit

DATA = Path(os.getenv("MIX_DATA", ".data")).resolve()
KEYS = Path(os.getenv("MIX_KEYS", str(DATA / "keys"))).resolve()
ARTIFACTS = DATA / "artifacts"
DATABASE_URL = os.getenv("DATABASE_URL")
if not DATABASE_URL:
    if not os.getenv("DB_PASSWORD_FILE"):
        raise RuntimeError("PostgreSQL DATABASE_URL or DB_PASSWORD_FILE is required")
    from urllib.parse import quote

    DATABASE_URL = (
        "postgresql+psycopg://mix:"
        + quote(Path(os.environ["DB_PASSWORD_FILE"]).read_text().strip())
        + "@db/mix"
    )
if not DATABASE_URL.startswith("postgresql+psycopg://"):
    raise RuntimeError("MIX agent requires a postgresql+psycopg DATABASE_URL")
RUNNER_URL = os.getenv("RUNNER_URL", "http://execution-runner:8001")
MCP_URL = os.getenv("MCP_URL", "http://mcp-runner:8002")
MCP_MANAGER_URL = os.getenv("MCP_MANAGER_URL", "http://mcp-manager:8003")
BROWSER_URL = os.getenv("BROWSER_URL", "http://browser-runner:8005")
BROWSER_PROVISIONER_URL = os.getenv("BROWSER_PROVISIONER_URL", "http://browser-provisioner:8006")
PUBLIC_ORIGIN = os.getenv("PUBLIC_ORIGIN", "http://localhost:8080").rstrip("/")
COOKIE_SECURE = PUBLIC_ORIGIN.startswith("https://")
MAX_UPLOAD = 20 * 1024 * 1024
# Bump when the tool-call probe semantics change so previously stored
# "unsupported"/"unknown" verdicts are not treated as authoritative.
TOOL_PROBE_VERSION = 2


def _origin_ips(hostname):
    """Resolve a hostname back to the set of IP addresses it may use.

    Accepts literal IPs too.  ``localhost`` resolves to its loopback
    address(es) instead of being rejected by ``ip_address`` only accepting
    literals.  Returns an empty set when nothing resolves.
    """
    if not hostname:
        return set()
    resolved = set()
    try:
        resolved.add(ip_address(hostname))
    except ValueError:
        pass
    try:
        infos = socket.getaddrinfo(hostname, None, proto=socket.IPPROTO_TCP)
    except socket.gaierror:
        infos = ()
    for _family, _type, _proto, _canonname, sockaddr in infos:
        ip = sockaddr[0]
        if ":" in ip:
            ip = ip.split("%")[0]
        try:
            resolved.add(ip_address(ip))
        except ValueError:
            continue
    return resolved


def allowed_origin(origin: str, *, scheme: str, host: str, forwarded: bool = False) -> bool:
    """Allow the configured public origin, plus a direct private/loopback origin.

    The private-IP exception supports opening the Docker-published port from a
    LAN browser before PUBLIC_ORIGIN has been configured.  Hostnames (``localhost``,
    a ``.local`` name, ...) are resolved to their addresses so they work exactly
    like the literal IP would.  It intentionally requires the browser origin and
    the request destination to resolve to the same address and port, rather than
    trusting arbitrary private-network origins.
    """
    if origin == PUBLIC_ORIGIN:
        return True
    if forwarded:
        return False
    try:
        source = urlsplit(origin)
        target = urlsplit("//" + host)
        if (
            source.scheme != scheme
            or source.path
            or source.query
            or source.fragment
            or source.username
            or source.password
            or target.username
            or target.password
        ):
            return False
        source_ips = _origin_ips(source.hostname or "")
        target_ips = _origin_ips(target.hostname or "")
        source_port = source.port or (443 if source.scheme == "https" else 80)
        target_port = target.port or (443 if scheme == "https" else 80)
    except ValueError:
        return False
    if source_port != target_port:
        return False
    for ip in source_ips & target_ips:
        if ip.is_private or ip.is_loopback:
            return True
    return False


def initialize():
    for path in (DATA, KEYS, ARTIFACTS):
        path.mkdir(parents=True, exist_ok=True)
    key = KEYS / "master.key"
    try:
        with key.open("xb") as f:
            os.chmod(key, 0o600)
            f.write(os.urandom(32))
    except FileExistsError:
        pass


def runner_token(kind="execution"):
    path = Path(os.getenv("RUNNER_KEYS", str(KEYS))) / (kind + ".token")
    return path.read_text().strip() if path.exists() else ""
