"""Create/update helpers for MCP connections.

Extracted from the router so the HTTP surface stays thin and the credential
rotation rule (replace the previous secret instead of orphaning it) lives in
one place.
"""

import json

from sqlalchemy import select

from mix_agent.api.common import own
from mix_agent.auth.security import store_optional_secret
from mix_agent.db.models import MCPConnection, Tool


def create_connection(db, owner_id, body):
    data = body.model_dump(exclude={"credentials"})
    if body.credentials:
        data["secret_id"] = store_optional_secret(
            db, owner_id, json.dumps(body.credentials), "mcp"
        )
    row = MCPConnection(owner_id=owner_id, data=data)
    db.add(row)
    db.flush()
    return row


def update_connection(db, owner_id, key, body):
    row = own(db, MCPConnection, key, owner_id)
    data = body.model_dump(exclude={"credentials"})
    data["secret_id"] = row.data.get("secret_id")
    if body.credentials is not None:
        data["secret_id"] = store_optional_secret(
            db,
            owner_id,
            json.dumps(body.credentials) if body.credentials else "",
            "mcp",
            row.data.get("secret_id"),
        )
    row.data = data
    # Connection edits invalidate all previously discovered definitions and grants.
    for tool in db.scalars(select(Tool).where(Tool.owner_id == owner_id)):
        if tool.data.get("source_ref") == key:
            tool.data = {**tool.data, "enabled": False}
    return row
