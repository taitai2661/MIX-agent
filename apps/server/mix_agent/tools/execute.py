import base64
import asyncio
import hashlib
import json
import httpx
import re
from bs4 import BeautifulSoup
from mix_agent import config
from mix_agent.db.models import Artifact, MCPConnection, Settings
from mix_agent.auth.security import read_secret
from mix_agent.memory import service as memory
from mix_agent.skills import service as skills
from mix_agent.tools.network import fetch_public
from sqlalchemy import select


async def runner_request(kind, path, payload, timeout=120):
    token = config.runner_token(kind)
    if not token:
        raise RuntimeError("Runner is not configured. Start the Docker deployment.")
    url = config.RUNNER_URL if kind == "execution" else config.MCP_URL
    async with httpx.AsyncClient(timeout=timeout, trust_env=False) as client:
        response = await client.post(url + path, json=payload, headers={"Authorization": "Bearer " + token})
        response.raise_for_status()
        return response.json()


def save_artifact(db, owner, content, name, mime="text/plain"):
    from mix_agent.db.models import uid

    key = uid()
    (config.ARTIFACTS / key).write_bytes(content)
    row = Artifact(
        id=key,
        owner_id=owner,
        data={
            "name": name,
            "mime": mime,
            "size": len(content),
            "sha256": hashlib.sha256(content).hexdigest(),
        },
    )
    db.add(row)
    db.flush()
    return {"artifact_id": key, **row.data}


def save_text_artifact(db, owner, name, mime, content):
    """Validate and persist a user-downloadable text artifact."""
    if not isinstance(name, str) or not name.strip() or name != name.strip():
        raise ValueError("Artifact filename is required")
    if len(name) > 255 or name in {".", ".."} or "/" in name or "\\" in name or ".." in name:
        raise ValueError("Artifact filename must be a single safe filename")
    if not re.fullmatch(r"[\w .()\-]+", name, re.UNICODE):
        raise ValueError("Artifact filename contains unsupported characters")
    if not isinstance(mime, str) or len(mime) > 120 or ";" in mime or not re.fullmatch(r"[\w.+-]+/[\w.+-]+", mime):
        raise ValueError("Artifact MIME type is invalid")
    if not isinstance(content, str) or not content:
        raise ValueError("Artifact content is required")
    if len(content.encode("utf-8")) > 1024 * 1024:
        raise ValueError("Artifact content is too large")
    return save_artifact(db, owner, content.encode("utf-8"), name, mime)


async def execute(db, run, tool, args):
    name = tool["model_name"]
    scopes = run.data["snapshot"].get("memory_scopes", ["user"])
    if tool["source"] == "mcp":
        conn = db.get(MCPConnection, tool["source_ref"])
        if not conn or not conn.data.get("enabled", True):
            raise ValueError("MCP connection is disabled")
        result = await runner_request(
            "mcp",
            "/call",
            {
                "connection": {
                    **conn.data,
                    "credentials": json.loads(read_secret(db, conn.data.get("secret_id")) or "{}"),
                },
                "tool": tool["remote_name"],
                "arguments": args,
                "run_id": run.id,
            },
        )
    elif tool["executor_ref"] == "execution":
        result = await runner_request(
            "execution", "/execute", {"name": name, "arguments": args, "run_id": run.id}
        )
        if "image_base64" in result:
            artifact = save_artifact(
                db, run.owner_id, base64.b64decode(result.pop("image_base64")), "screenshot.png", "image/png"
            )
            result["artifact"] = artifact
    elif name == "web_fetch":
        html = await fetch_public(args["url"])
        soup = BeautifulSoup(html, "html.parser")
        for tag in soup(["script", "style", "nav"]):
            tag.decompose()
        result = {"url": args["url"], "text": soup.get_text(" ", strip=True)}
    elif name == "web_search":
        setting = db.scalar(select(Settings).where(Settings.owner_id == run.owner_id))
        data = setting.data if setting else {}
        count = max(1, min(20, data.get("web_search_count", 5)))
        if data.get("web_search_backend", "ddgs") == "brave":
            key = read_secret(db, data.get("brave_secret_id"))
            if not key:
                raise ValueError("Brave Search API Keyが未設定です")
            async with httpx.AsyncClient(timeout=20) as client:
                response = await client.get("https://api.search.brave.com/res/v1/web/search", params={"q": args["query"], "count": count}, headers={"X-Subscription-Token": key})
                response.raise_for_status()
                result = {"results": response.json().get("web", {}).get("results", [])}
        else:
            from ddgs import DDGS
            rows = await asyncio.wait_for(asyncio.to_thread(lambda: list(DDGS().text(args["query"], max_results=count))), timeout=20)
            result = {"results": [{"title": row.get("title", ""), "url": row.get("href", ""), "description": row.get("body", "")} for row in rows]}
    elif name == "memory_search":
        searched = memory.search(db, run.owner_id, args.get("query", ""), scopes, debug=bool(args.get("debug")))
        result = searched if args.get("debug") else {"memories": searched}
    elif name.startswith("memory_"):
        if not scopes:
            raise ValueError("Memory is disabled for this agent")
        result = memory.change(
            db,
            run.owner_id,
            args.get("content"),
            args.get("id"),
            name == "memory_delete",
            scope=scopes[0],
            source_run=run.id,
            scopes=scopes,
            importance=args.get("importance", 2),
            category=args.get("category"),
            pinned=args.get("pinned", False),
            confidence=args.get("confidence"),
            salience=args.get("salience"),
            entities=args.get("entities"),
            concepts=args.get("concepts"),
        )
    elif name == "skill_search":
        result = {"skills": skills.search(db, run.owner_id, args.get("query", ""), run.data["snapshot"].get("skill_ids"))}
    elif name == "skill_add":
        result = skills.change(db, run.owner_id, name=args["name"], description=args.get("description", ""), content=args["content"], source_run=run.id)
    elif name == "skill_update":
        result = skills.change(db, run.owner_id, name=args.get("name"), description=args.get("description"), content=args.get("content"), skill_id=args["id"], source_run=run.id)
    elif name == "update_plan":
        result = {"steps": args["steps"]}
    elif name == "create_artifact":
        result = {"artifact": save_text_artifact(db, run.owner_id, args["name"], args["mime"], args["content"])}
    else:
        raise ValueError("Unknown tool")
    raw = json.dumps(result, ensure_ascii=False)
    if len(raw) > 16000:
        artifact = save_artifact(db, run.owner_id, raw.encode(), "tool-result.json", "application/json")
        return {"excerpt": raw[:16000], "truncated": True, "artifact": artifact}
    return result
