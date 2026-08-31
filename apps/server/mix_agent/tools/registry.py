import hashlib
import json
from pathlib import PurePosixPath
from jsonschema import Draft202012Validator
from sqlalchemy import select
from mix_agent.db.models import Tool, Permission


def definition(name, description, properties=None, required=None, permission="allow", executor="execution"):
    return {
        "id": name,
        "model_name": name,
        "description": description,
        "source": "builtin",
        "version": "1",
        "executor_ref": executor,
        "risk": "read" if permission == "allow" else "write",
        "default_permission": permission,
        "input_schema": {
            "type": "object",
            "properties": properties or {},
            "required": required or [],
            "additionalProperties": False,
        },
        "output_schema": {"type": "object"},
    }


S = {"type": "string", "maxLength": 100000}
PATH = {"type": "string", "maxLength": 1024}
BUILTINS = [
    definition("web_search", "Search the public web", {"query": S}, ["query"], executor="builtin"),
    definition("web_fetch", "Read a public web page", {"url": S}, ["url"], executor="builtin"),
    definition("files_list", "List files in /workspace", {"path": PATH}),
    definition("read_file", "Read a UTF-8 workspace file", {"path": PATH}, ["path"]),
    definition(
        "write_file",
        "Create or replace a workspace file",
        {"path": PATH, "content": S},
        ["path", "content"],
        "ask",
    ),
    definition(
        "edit_file",
        "Replace exact text in a workspace file",
        {"path": PATH, "old": S, "new": S},
        ["path", "old", "new"],
        "ask",
    ),
    definition("delete_file", "Delete a workspace file (not directories)", {"path": PATH}, ["path"], "ask"),
    definition("search_files", "Search workspace text files", {"query": S}, ["query"]),
    definition(
        "run_terminal",
        "Run shell code in the dedicated workspace; no host access",
        {"command": S, "background": {"type": "boolean"}},
        ["command"],
        "ask",
    ),
    definition("process_list", "List workspace processes"),
    definition("process_stop", "Stop a workspace process group", {"process_id": S}, ["process_id"], "ask"),
    definition("browser_open", "Open a permitted public URL", {"url": S}, ["url"], "ask"),
    definition("browser_click", "Click a Playwright locator", {"selector": S}, ["selector"], "ask"),
    definition(
        "browser_type", "Fill a Playwright locator", {"selector": S, "text": S}, ["selector", "text"], "ask"
    ),
    definition("browser_read", "Read the current browser page"),
    definition("browser_screenshot", "Capture the current browser page"),
    definition("memory_search", "Search permitted long-term memory", {"query": S}, executor="builtin"),
    definition(
        "memory_add",
        "Remember a preference or decision, never credentials",
        {"content": S},
        ["content"],
        executor="builtin",
    ),
    definition(
        "memory_update",
        "Update existing memory, retaining history",
        {"id": S, "content": S},
        ["id", "content"],
        executor="builtin",
    ),
    definition("skill_search", "Search reusable work instructions", {"query": S}, executor="builtin"),
    definition("skill_add", "Save a reusable verified workflow. Never include credentials.", {"name": {"type": "string", "maxLength": 100}, "description": {"type": "string", "maxLength": 1000}, "content": S}, ["name", "content"], executor="builtin"),
    definition("skill_update", "Update a reusable workflow, retaining history", {"id": S, "name": {"type": "string", "maxLength": 100}, "description": {"type": "string", "maxLength": 1000}, "content": S}, ["id"], executor="builtin"),
    definition("memory_delete", "Delete existing memory with approval", {"id": S}, ["id"], "ask", "builtin"),
    definition(
        "update_plan",
        "Publish a short user-facing task checklist, not private reasoning",
        {"steps": {"type": "array", "maxItems": 20, "items": S}},
        ["steps"],
        executor="builtin",
    ),
]


def registry(db, owner):
    items = {t["id"]: dict(t) for t in BUILTINS}
    for row in db.scalars(select(Tool).where(Tool.owner_id == owner)):
        if row.data.get("enabled", True):
            items[row.id] = {**row.data, "id": row.id}
    return items


def fingerprint(tool):
    return hashlib.sha256(json.dumps(tool, sort_keys=True).encode()).hexdigest()


def call_scope(tool, args):
    if tool["executor_ref"] == "execution":
        return {"workspace": "/workspace"}
    if tool["source"] == "mcp":
        return {"connection_id": tool["source_ref"]}
    return {"tool": tool["id"]}


def permission(db, run, tool, args):
    Draft202012Validator(tool["input_schema"]).validate(args)
    snapshot = run.data["snapshot"]
    if tool["id"] not in snapshot["tool_ids"]:
        return "deny"
    if "path" in args:
        path = PurePosixPath(args["path"])
        if ".." in path.parts or (path.is_absolute() and path.parts[:2] != ("/", "workspace")):
            return "deny"
    rules = list(db.scalars(select(Permission).where(Permission.owner_id == run.owner_id)))
    matching = [
        r.data
        for r in rules
        if r.data.get("tool_id") == tool["id"]
        and r.data.get("agent_id", "") == snapshot.get("agent_id", "")
        and r.data.get("scope") == call_scope(tool, args)
        and r.data.get("tool_version") == fingerprint(tool)
    ]
    if any(x["permission"] == "deny" for x in matching):
        return "deny"
    return matching[-1]["permission"] if matching else tool["default_permission"]
