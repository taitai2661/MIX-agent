import os
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


def allowed_origin(origin: str, *, scheme: str, host: str, forwarded: bool = False) -> bool:
    """Allow the configured public origin, plus a direct private-IP origin.

    The private-IP exception supports opening the Docker-published port from a
    LAN browser before PUBLIC_ORIGIN has been configured.  It intentionally
    requires the browser origin to be the exact request destination, rather
    than trusting arbitrary private-network origins.
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
        source_ip = ip_address(source.hostname or "")
        target_ip = ip_address(target.hostname or "")
        source_port = source.port or (443 if source.scheme == "https" else 80)
        target_port = target.port or (443 if scheme == "https" else 80)
    except ValueError:
        return False
    return (
        (source_ip.is_private or source_ip.is_loopback)
        and source_ip == target_ip
        and source_port == target_port
    )


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
