"""Single writer for the egress proxy policy file.

Both the settings API and backup restore produce the same document, and it is
consumed by a separate container (`deploy/egress/proxy.py`). Keeping the format
in one place means a new field (such as the explicit `mode`) cannot be added to
one writer and forgotten in the other.
"""

import json
import logging
import os
import tempfile
from pathlib import Path

from mix_agent import config
from mix_agent.api.error_codes import APIError

LOGGER = logging.getLogger(__name__)


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
    # A fixed temp name let the settings route and a backup restore overwrite
    # each other's half-written file. A unique file in the same directory keeps
    # ``os.replace`` atomic and lets either writer fail without leaving a stale
    # ``policy.tmp`` behind.
    handle = tempfile.NamedTemporaryFile(
        "w",
        dir=directory,
        prefix="policy.",
        suffix=".tmp",
        delete=False,
        encoding="utf-8",
    )
    temp = Path(handle.name)
    try:
        # The proxy container may run under a different UID in a custom
        # deployment; the policy is not secret, so keep the historical 0644
        # readability instead of NamedTemporaryFile's 0600.
        os.chmod(temp, 0o644)
        with handle:
            json.dump(
                {
                    "mode": "restricted" if domains else "public_web",
                    "allowed_domains": domains,
                },
                handle,
            )
            handle.flush()
            os.fsync(handle.fileno())
        temp.replace(target)
    finally:
        temp.unlink(missing_ok=True)
    return target


def write_policy_or_503(domains) -> Path:
    """Write the policy, translating a storage failure into an API error.

    The settings row is already committed at this point; reporting the failure
    is safer than claiming success while container egress stays stale.
    """
    try:
        return write_policy(domains)
    except OSError as exc:
        raise APIError(503, "egress.policy_write_failed") from exc


def write_policy_best_effort(domains) -> bool:
    """Write the policy without raising when the durable state is already saved.

    A restore commits the database before this runs. A failed file write must
    not make the caller report failure (or, worse, leave the recovery journal
    behind so the next boot rolls the restore back); the next start reconciles
    the file from the database.
    """
    try:
        write_policy(domains)
    except OSError:
        LOGGER.exception("egress policy could not be written; it will be reconciled on the next start")
        return False
    return True


def reconcile_from_db() -> bool:
    """Rewrite the proxy policy from the saved settings.

    The database is the source of truth. This repairs a file left stale by a
    failed settings update or restore, and keeps the two writers from drifting.
    """
    try:
        # Local imports keep this module importable from lightweight scripts
        # and avoid a config/db import cycle at module load time.
        from mix_agent.db.models import Settings
        from mix_agent.db.session import SessionLocal

        with SessionLocal() as db:
            row = db.get(Settings, "settings")
            if row is None:
                return False
            return write_policy_best_effort(row.data.get("allowed_domains", []))
    except Exception:  # noqa: BLE001 - startup must not fail because policy repair failed
        LOGGER.exception("could not reconcile the egress policy from settings")
        return False
