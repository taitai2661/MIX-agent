"""Guard against new sync-DB work inside async endpoints.

`get_db()` yields a synchronous SQLAlchemy `Session`.  An `async def` endpoint
that calls it runs those queries on the event loop, so a slow query stalls
every SSE stream in the single-process deployment.  Unblocking an endpoint
means moving its services to an async session, or to a `def` endpoint when its
awaits are only the in-memory throttle; until then this test freezes the list so
new code cannot make the problem worse.

When you convert one, delete its name from the set below.
"""

import ast
from pathlib import Path

from mix_agent.api import routes as routes_module

API_DIR = Path(routes_module.__file__).parent

KNOWN_ASYNC_DB_ENDPOINTS = {
    "add_memory",
    "add_provider",
    "browser_install",
    "browser_manual",
    "browser_return",
    "browser_update",
    "cancel",
    "delete_conversation",
    "edit_memory",
    "edit_message",
    "edit_provider",
    "import_skill_archive",
    "import_skill_directory",
    "install_registry_mcp",
    "mcp_action",
    "mcp_oauth_callback",
    "mcp_runtime_action",
    "provider_action",
    "reinstall_mcp",
    "send_message",
    "start_mcp_oauth",
    "uninstall_mcp",
    "update_registry_mcp",
    "upload",
    "verify_model_tools",
}


def _async_db_endpoints() -> set[str]:
    found: set[str] = set()
    for path in sorted(API_DIR.glob("routes*.py")):
        tree = ast.parse(path.read_text(encoding="utf-8"))
        for node in tree.body:
            if not isinstance(node, ast.AsyncFunctionDef):
                continue
            if not any(isinstance(decorator, ast.Call) for decorator in node.decorator_list):
                continue
            uses_db = any(
                isinstance(item, ast.Attribute) and isinstance(item.value, ast.Name) and item.value.id == "db"
                for item in ast.walk(node)
            )
            awaitable = any(
                isinstance(item, (ast.Await, ast.AsyncFor, ast.AsyncWith)) for item in ast.walk(node)
            )
            if uses_db and awaitable:
                found.add(node.name)
    return found


def test_no_new_async_endpoint_uses_the_sync_session():
    new = sorted(_async_db_endpoints() - KNOWN_ASYNC_DB_ENDPOINTS)
    assert not new, (
        "These async endpoints use the sync SQLAlchemy session and will block the "
        f"event loop: {new}. Use a def endpoint, or move the DB work behind an "
        "async service, before adding them."
    )
