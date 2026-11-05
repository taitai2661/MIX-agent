"""Validation for the user-supplied SearXNG endpoint.

Kept out of the settings router so the route stays a thin HTTP surface and the
same rules can be reused by the search tool.
"""

from urllib.parse import urlsplit

from mix_agent.tools.network import public_address_sync


def validate_searxng_url(raw: str) -> str:
    """Return the normalized URL or raise ``ValueError`` with a stable reason."""
    url = raw.strip()
    parsed = urlsplit(url)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.port not in (None, 443)
    ):
        raise ValueError("SearXNGのURLは公開HTTPSエンドポイントを指定してください")
    if len(url) > 500:
        raise ValueError("SearXNGのURLが長すぎます")
    try:
        public_address_sync(parsed.hostname, 443)
    except (OSError, ValueError) as exc:
        raise ValueError("SearXNGのURLは公開されたホスト名を指定してください") from exc
    return url.rstrip("/")
