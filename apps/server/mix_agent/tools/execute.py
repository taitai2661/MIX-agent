import asyncio
import base64
import hashlib
import json
import re
import time

import httpx
from bs4 import BeautifulSoup
from sqlalchemy import select

from mix_agent import config
from mix_agent.auth.security import read_secret, store_secret
from mix_agent.db.models import Agent, Artifact, Conversation, MCPConnection, ScheduledJob, Settings, now
from mix_agent.mcp import oauth as mcp_oauth
from mix_agent.memory import service as memory
from mix_agent.skills import service as skills
from mix_agent.tools.network import fetch_public


async def runner_request(kind, path, payload, timeout=120):
    token = config.runner_token(kind)
    if not token:
        raise RuntimeError("Runner is not configured. Start the Docker deployment.")
    url = {"execution": config.RUNNER_URL, "mcp": config.MCP_URL, "manager": config.MCP_MANAGER_URL,
           "browser": config.BROWSER_URL, "browser-provisioner": config.BROWSER_PROVISIONER_URL}[kind]
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
        credentials = json.loads(read_secret(db, conn.data.get("secret_id")) or "{}")
        oauth = credentials.get("oauth")
        if oauth and int(oauth.get("obtained_at", 0)) + int(oauth.get("expires_in", 0)) <= int(time.time()) + 60:
            try:
                oauth = await mcp_oauth.refresh(oauth, credentials.get("oauth_registration", {}))
                credentials = {**credentials, "oauth": oauth, "headers": {**credentials.get("headers", {}), "Authorization": "Bearer " + oauth["access_token"]}}
                conn.data = {**conn.data, "secret_id": store_secret(db, conn.owner_id, json.dumps(credentials), "mcp")}
                db.commit()
            except Exception:
                conn.data = {**conn.data, "state": "needs_authorization", "authorization_required": True, "enabled": False}
                db.commit()
                raise ValueError("MCPの再認証が必要です")
        if conn.data.get("runtime", {}).get("driver") in ("oci", "npm", "pypi"):
            result = await runner_request(
                "manager",
                f"/v1/call/{conn.id}",
                {"credentials": credentials, "arguments": conn.data.get("runtime_arguments", []), "tool": tool["remote_name"], "arguments_value": args},
            )
        else:
            result = await runner_request(
                "mcp",
                "/call",
                {"connection": {**conn.data, "credentials": credentials}, "tool": tool["remote_name"], "arguments": args, "run_id": run.id},
            )
    elif tool["executor_ref"] == "execution":
        setting = db.scalar(select(Settings).where(Settings.owner_id == run.owner_id))
        browser_settings = setting.data if setting else {}
        if name.startswith("browser_"):
            try:
                status = await runner_request("browser-provisioner", "/status", {}, timeout=5)
            except Exception as error:
                raise ValueError("Browserの導入状態を確認できません。設定画面から再試行してください。") from error
            if status.get("status") != "ready":
                raise ValueError("Browserはまだ利用できません（%s）。設定画面から導入または再試行してください。" % status.get("status", "not_installed"))
        result = await runner_request(
            "browser" if name.startswith("browser_") else "execution", "/execute", {"name": name, "arguments": args, "run_id": run.id,
                                      "browser_settings": browser_settings}
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
    elif name == "schedule_list":
        from mix_agent.schedules import next_at

        jobs = []
        for job in db.scalars(select(ScheduledJob).where(ScheduledJob.owner_id == run.owner_id).order_by(ScheduledJob.created_at.desc())):
            data = dict(job.data)
            try:
                data["next_at"] = next_at(data["cron"], data["timezone"]).isoformat() if data.get("enabled") else None
            except ValueError:
                data["next_at"] = None
            jobs.append({"id": job.id, **data})
        result = {"jobs": jobs}
    elif name in ("schedule_create", "schedule_update", "schedule_delete", "schedule_run"):
        from mix_agent.schedules import claim, next_at

        def owned_job(key):
            job = db.get(ScheduledJob, key)
            if not job or job.owner_id != run.owner_id:
                raise ValueError("Scheduled job was not found")
            return job

        def validate_target(data):
            model = Agent if data["target_type"] == "agent" else Conversation
            target = db.get(model, data["target_id"])
            if not target or target.owner_id != run.owner_id:
                raise ValueError("Scheduled job target was not found")

        if name == "schedule_create":
            data = {"name": args["name"], "target_type": args["target_type"], "target_id": args["target_id"], "prompt": args["prompt"], "cron": args["cron"], "timezone": args.get("timezone", "Asia/Tokyo"), "enabled": args.get("enabled", True), "catch_up": args.get("catch_up", False)}
            validate_target(data)
            next_at(data["cron"], data["timezone"])
            job = ScheduledJob(owner_id=run.owner_id, data=data)
            db.add(job); db.flush()
            result = {"id": job.id, "created": True, **data}
        elif name == "schedule_update":
            job = owned_job(args["id"])
            data = {**job.data, **{key: value for key, value in args.items() if key != "id"}}
            validate_target(data)
            next_at(data["cron"], data["timezone"])
            job.data = data
            result = {"id": job.id, "updated": True, **data}
        elif name == "schedule_delete":
            job = owned_job(args["id"])
            job_id = job.id
            db.delete(job)
            result = {"id": job_id, "deleted": True}
        else:
            job = owned_job(args["id"])
            scheduled = claim(db, job, now())
            if not scheduled:
                raise ValueError("This scheduled run has already started")
            from mix_agent.api.routes import enqueue_scheduled_run
            from mix_agent.runs.engine import launch

            scheduled_run = enqueue_scheduled_run(db, run.owner_id, job, scheduled)
            # The launched run uses a separate session, so make its job and run
            # visible before handing it to the background engine.
            db.commit()
            launch(scheduled_run.id)
            result = {"id": job.id, "scheduled_run_id": scheduled.id, "run_id": scheduled_run.id, "started": True}
    elif name == "create_artifact":
        result = {"artifact": save_text_artifact(db, run.owner_id, args["name"], args["mime"], args["content"])}
    else:
        raise ValueError("Unknown tool")
    raw = json.dumps(result, ensure_ascii=False)
    if len(raw) > 16000:
        artifact = save_artifact(db, run.owner_id, raw.encode(), "tool-result.json", "application/json")
        return {"excerpt": raw[:16000], "truncated": True, "artifact": artifact}
    return result
