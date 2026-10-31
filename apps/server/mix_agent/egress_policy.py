"""Single writer for the egress proxy policy file.

Both the settings API and backup restore produce the same document, and it is
consumed by a separate container (`deploy/egress/proxy.py`). Keeping the format
in one place means a new field (such as the explicit `mode`) cannot be added to
one writer and forgotten in the other.
"""

import json
from pathlib import Path

from mix_agent import config


def write_policy(domains) -> Path:
    """Write ``{mode, allowed_domains}`` atomically and return the path.

    An empty allow-list keeps the historical "any public host" behaviour, but it
    is now recorded as ``mode: public_web`` rather than inferred from the empty
    list. A single entry switches the proxy to exact-match ``restricted``.
    """
    domains = [str(domain) for domain in domains]
    directory = config.DATA / "egress"
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / "policy.json"
    temp = directory / "policy.tmp"
    temp.write_text(
        json.dumps(
            {
                "mode": "restricted" if domains else "public_web",
                "allowed_domains": domains,
            }
        )
    )
    temp.replace(target)
    return target
