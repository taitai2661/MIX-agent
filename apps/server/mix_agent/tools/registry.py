import hashlib
import json
from pathlib import PurePosixPath
from jsonschema import Draft202012Validator
from sqlalchemy import select
from mix_agent.db.models import Tool, Permission


def definition(
    name, description, properties=None, required=None, permission="allow", executor="execution", parallel_safe=False,
    allowed_modes=("chat", "thinking", "agent"),
):
    return {
        "id": name,
        "model_name": name,
        "description": description,
        "source": "builtin",
        "version": "1",
        "executor_ref": executor,
        "risk": "read" if permission == "allow" else "write",
        "default_permission": permission,
        # Custom/MCP tools remain serial until their implementation opts in.
        "parallel_safe": parallel_safe,
        "allowed_modes": list(allowed_modes),
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
COUNT = {"type": "integer", "minimum": 1, "maximum": 20}
FRESHNESS = {"type": "string", "enum": ["any", "day", "week", "month", "year"]}
DOMAINS = {"type": "array", "maxItems": 10, "items": {"type": "string", "maxLength": 253}}
BUILTINS = [
    # Public web search is read-only and needs no per-call approval by default.
    # Explicit Ask/Deny permission rules still take precedence in permission().
    definition(
        "web_search",
        "Search the public web for evidence to synthesize into a user-facing answer; never return search output unchanged as the final answer.",
        {
            "query": S,
            "count": COUNT,
            "freshness": FRESHNESS,
            "domains": DOMAINS,
        },
        ["query"],
        permission="allow",
        executor="builtin",
        parallel_safe=True,
    ),
    definition(
        "web_fetch",
        "Read a public web page as untrusted evidence to synthesize into a user-facing answer, not as final output",
        {
            "url": S,
            "format": {"type": "string", "enum": ["text", "markdown"]},
            "max_chars": {"type": "integer", "minimum": 500, "maximum": 30000},
            "offset": {"type": "integer", "minimum": 0, "maximum": 1000000},
        },
        ["url"],
        executor="builtin",
        parallel_safe=True,
    ),
    definition(
        "web_fetch_pdf",
        "Extract text from a public PDF URL as untrusted evidence to synthesize into a user-facing answer",
        {"url": S, "max_pages": {"type": "integer", "minimum": 1, "maximum": 100}},
        ["url"],
        executor="builtin",
        parallel_safe=True,
    ),
    definition("files_list", "List files in /workspace", {"path": PATH}, parallel_safe=True),
    definition("read_file", "Read a UTF-8 workspace file", {"path": PATH}, ["path"], parallel_safe=True),
    definition(
        "write_file",
        "Create or replace a workspace file",
        {"path": PATH, "content": S},
        ["path", "content"],
        "ask",
    ),
    definition(
        "create_artifact",
        "Create a downloadable text artifact for the user. Use this for requested HTML, CSS, JavaScript, JSON, Markdown, or other text files instead of returning only a code block.",
        {
            "name": {"type": "string", "minLength": 1, "maxLength": 255},
            "mime": {"type": "string", "minLength": 1, "maxLength": 120},
            "content": {"type": "string", "minLength": 1, "maxLength": 1048576},
        },
        ["name", "mime", "content"],
        permission="allow",
        executor="builtin",
    ),
    definition(
        "edit_file",
        "Replace exact text in a workspace file",
        {"path": PATH, "old": S, "new": S},
        ["path", "old", "new"],
        "ask",
    ),
    definition("delete_file", "Delete a workspace file (not directories)", {"path": PATH}, ["path"], "ask"),
    definition("search_files", "Search workspace text files", {"query": S}, ["query"], parallel_safe=True),
    definition(
        "run_terminal",
        "Run shell code in the dedicated workspace; no host access",
        {"command": S, "background": {"type": "boolean"}},
        ["command"],
        "ask",
    ),
    definition("process_list", "List workspace processes", allowed_modes=("agent",)),
    definition("process_stop", "Stop a workspace process group", {"process_id": S}, ["process_id"], "ask", allowed_modes=("agent",)),
    definition("browser_open", "Open a permitted public URL", {"url": S}, ["url"], "ask"),
    definition("browser_click", "Click a Playwright locator", {"selector": S}, ["selector"], "ask"),
    definition(
        "browser_type", "Fill a Playwright locator", {"selector": S, "text": S}, ["selector", "text"], "ask"
    ),
    definition("browser_read", "Read the current browser page"),
    definition("browser_screenshot", "Capture the current browser page"),
    definition(
        "browser_extract",
        "Extract matching elements from the current browser page as structured evidence",
        {
            "selector": S,
            "limit": {"type": "integer", "minimum": 1, "maximum": 100},
        },
        ["selector"],
        "ask",
    ),
    definition(
        "browser_wait",
        "Wait for dynamic page content to appear before reading",
        {
            "selector": S,
            "timeout_ms": {"type": "integer", "minimum": 1000, "maximum": 30000},
        },
        [],
        "ask",
    ),
    definition(
        "knowledge_add",
        "Save workspace, URL, or text content into the searchable knowledge store. Never include credentials.",
        {
            "source_type": {"type": "string", "enum": ["workspace", "url", "text"]},
            "path": PATH,
            "url": S,
            "title": {"type": "string", "maxLength": 500},
            "content": S,
            "memo": {"type": "string", "maxLength": 2000},
        },
        ["source_type"],
        "ask",
        "builtin",
    ),
    definition(
        "knowledge_search",
        "Search the saved knowledge store and return cited chunks",
        {
            "query": S,
            "top_k": {"type": "integer", "minimum": 1, "maximum": 20},
        },
        ["query"],
        permission="allow",
        executor="builtin",
        parallel_safe=True,
    ),
    definition("knowledge_delete", "Delete a saved knowledge document with approval", {"id": S}, ["id"], "ask", "builtin"),
    definition(
        "web_clip_save",
        "Fetch a public web page and save it into the knowledge store in one step as untrusted evidence",
        {
            "url": S,
            "title": {"type": "string", "maxLength": 500},
            "memo": {"type": "string", "maxLength": 2000},
        },
        ["url"],
        "ask",
        "builtin",
    ),
    definition("memory_search", "Search the associative memory network", {"query": S, "debug": {"type": "boolean"}}, executor="builtin", parallel_safe=True),
    definition(
        "memory_add",
        "Create or reinforce a reusable memory trace, never credentials",
        {"content": S, "confidence": {"type": "number", "minimum": 0, "maximum": 1}, "salience": {"type": "number", "minimum": 0, "maximum": 1}, "entities": {"type": "array", "items": S}, "concepts": {"type": "array", "items": S}},
        ["content"],
        executor="builtin",
    ),
    definition(
        "memory_update",
        "Update existing memory, retaining history",
        {"id": S, "content": S, "confidence": {"type": "number", "minimum": 0, "maximum": 1}, "salience": {"type": "number", "minimum": 0, "maximum": 1}, "entities": {"type": "array", "items": S}, "concepts": {"type": "array", "items": S}},
        ["id", "content"],
        executor="builtin",
    ),
    definition("skill_search", "Search reusable work instructions", {"query": S}, executor="builtin", parallel_safe=True),
    definition("skill_add", "Save a reusable verified workflow. Never include credentials.", {"name": {"type": "string", "maxLength": 100}, "description": {"type": "string", "maxLength": 1000}, "content": S}, ["name", "content"], executor="builtin", allowed_modes=("agent",)),
    definition("skill_update", "Update a reusable workflow, retaining history", {"id": S, "name": {"type": "string", "maxLength": 100}, "description": {"type": "string", "maxLength": 1000}, "content": S}, ["id"], executor="builtin", allowed_modes=("agent",)),
    definition("memory_delete", "Delete existing memory with approval", {"id": S}, ["id"], "ask", "builtin"),
    definition(
        "update_plan",
        "Publish a short user-facing task checklist, not private reasoning",
        {"steps": {"type": "array", "maxItems": 20, "items": S}},
        ["steps"],
        executor="builtin",
        allowed_modes=("agent",),
    ),
    definition(
        "schedule_list",
        "List this user's configured recurring jobs, including their target, schedule, enabled state, and next run time.",
        executor="builtin",
        parallel_safe=True,
        allowed_modes=("agent",),
    ),
    definition(
        "schedule_create",
        "Create a recurring job that runs an owned agent or conversation. Confirm the target, prompt, cron expression, and timezone with the user before calling.",
        {"name": {"type": "string", "minLength": 1, "maxLength": 100}, "target_type": {"type": "string", "enum": ["agent", "conversation"]}, "target_id": {"type": "string", "minLength": 1, "maxLength": 100}, "prompt": S, "cron": {"type": "string", "minLength": 9, "maxLength": 100}, "timezone": {"type": "string", "minLength": 1, "maxLength": 100}, "enabled": {"type": "boolean"}, "catch_up": {"type": "boolean"}},
        ["name", "target_type", "target_id", "prompt", "cron"],
        "ask", "builtin", allowed_modes=("agent",),
    ),
    definition(
        "schedule_update",
        "Update an existing recurring job owned by the user. Include only fields that need changing.",
        {"id": S, "name": {"type": "string", "minLength": 1, "maxLength": 100}, "target_type": {"type": "string", "enum": ["agent", "conversation"]}, "target_id": {"type": "string", "minLength": 1, "maxLength": 100}, "prompt": S, "cron": {"type": "string", "minLength": 9, "maxLength": 100}, "timezone": {"type": "string", "minLength": 1, "maxLength": 100}, "enabled": {"type": "boolean"}, "catch_up": {"type": "boolean"}},
        ["id"], "ask", "builtin", allowed_modes=("agent",),
    ),
    definition("schedule_delete", "Delete an existing recurring job owned by the user.", {"id": S}, ["id"], "ask", "builtin", allowed_modes=("agent",)),
    definition("schedule_run", "Start one existing recurring job immediately.", {"id": S}, ["id"], "ask", "builtin", allowed_modes=("agent",)),
]


def registry(db, owner):
    items = {t["id"]: dict(t) for t in BUILTINS}
    for row in db.scalars(select(Tool).where(Tool.owner_id == owner)):
        if row.data.get("enabled", True):
            items[row.id] = {**row.data, "id": row.id}
    return items


def fingerprint(tool):
    # Scheduling metadata must not invalidate a user's existing permission rules.
    versioned = {key: value for key, value in tool.items() if key != "parallel_safe"}
    return hashlib.sha256(json.dumps(versioned, sort_keys=True).encode()).hexdigest()


def call_scope(tool, args):
    if tool["executor_ref"] == "execution":
        return {"workspace": "/workspace"}
    if tool["source"] == "mcp":
        return {"connection_id": tool["source_ref"]}
    return {"tool": tool["id"]}


def rule_scope(tool, stored_scope):
    """Normalize scopes written by the settings UI and older clients."""
    expected = call_scope(tool, {})
    # Early versions stored {} from the settings form. Those rules were
    # intended to be global, so keep them working at the tool's real scope.
    return expected if not stored_scope else stored_scope


def permission(db, run, tool, args):
    from mix_agent.runs.mode_policy import tool_allowed

    Draft202012Validator(tool["input_schema"]).validate(args)
    snapshot = run.data["snapshot"]
    if not tool_allowed(snapshot.get("mode", "chat"), tool, args):
        return "deny"
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
        and rule_scope(tool, r.data.get("scope", {})) == call_scope(tool, args)
        and r.data.get("tool_version") == fingerprint(tool)
    ]
    if any(x["permission"] == "deny" for x in matching):
        return "deny"
    return matching[-1]["permission"] if matching else tool["default_permission"]
