"""MCP OAuth state cleanup.

The transient client secret is stored in the ``secrets`` table and referenced
from ``MCPAuthState.data``. Deleting the state without its secret (the callback
or the expiry sweep) left decryptable rows behind.
"""

import json
from datetime import timedelta

from sqlalchemy import select

from mix_agent.auth.security import store_secret
from mix_agent.db.models import MCPAuthState, Secret, now


def replace_connection_secret(db, owner, credentials, previous_id):
    """Store OAuth credentials, replacing the connection's previous secret."""
    return store_secret(db, owner, json.dumps(credentials), "mcp", previous_id)


def create_transient_secret(db, owner, verifier, client_secret):
    """Store the PKCE verifier + client secret for one authorization state."""
    return store_secret(
        db,
        owner,
        json.dumps({"verifier": verifier, "client_secret": client_secret}),
        "mcp-oauth-state",
    )


def purge_expired_auth_states(db) -> None:
    for stale in db.scalars(select(MCPAuthState).where(MCPAuthState.created_at < now() - timedelta(hours=1))):
        delete_auth_state(db, stale)


def delete_auth_state(db, auth) -> None:
    secret_id = (auth.data or {}).get("secret_id")
    if secret_id:
        secret_row = db.get(Secret, secret_id)
        if secret_row:
            db.delete(secret_row)
    db.delete(auth)
