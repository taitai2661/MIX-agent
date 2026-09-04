import asyncio
import base64
import hashlib
import json
import re
import time
from urllib.parse import urlsplit

import httpx
from bs4 import BeautifulSoup
from sqlalchemy import select

from mix_agent import config
from mix_agent.auth.security import read_secret, store_secret
from mix_agent.db.models import Agent, Artifact, Conversation, MCPConnection, ScheduledJob, Settings, now
from mix_agent.knowledge import service as knowledge
from mix_agent.mcp import oauth as mcp_oauth
from mix_agent.memory import service as memory
from mix_agent.skills import service as skills
from mix_agent.tools.network import fetch_public, fetch_public_bytes

DDGS_TIMELIMIT = {"day": "d", "week": "w", "month": "m", "year": "y"}
BRAVE_FRESHNESS = {"day": "pd", "week": "pw", "month": "pm", "year": "py"}
TAVILY_TIME_RANGE = {"day": "day", "week": "week", "month": "month", "year": "year"}


def _sanitize_domains(domains):
    from urllib.parse import urlsplit

    cleaned = []
    for entry in domains or []:
        text = str(entry or "").strip().lower()
        if not text:
            continue
        if "://" in text:
            text = urlsplit(text).hostname or text
        text = text.strip().lstrip(".")[:253]
        if text and all(part and len(part) <= 63 for part in text.split(".")):
            cleaned.append(text)
    return list(dict.fromkeys(cleaned))[:10]


def _apply_domains(query, domains):
    cleaned = _sanitize_domains(domains)
    if not cleaned:
        return query
    return query + " " + " ".join("site:" + domain for domain in cleaned)


def _page_text(html):
    """Extract title, headings, links, and body text from HTML."""
    soup = BeautifulSoup(html, "html.parser")
    title = (soup.title.string or "").strip() if soup.title and soup.title.string else ""
    description = ""
    meta = soup.find("meta", attrs={"name": "description"}) or soup.find("meta", attrs={"property": "og:description"})
    if meta and meta.get("content"):
        description = meta["content"].strip()[:2000]
    published = ""
    for selector in ({"property": "article:published_time"}, {"name": "date"}, {"name": "publish_date"}):
        tag = soup.find("meta", attrs=selector)
        if tag and tag.get("content"):
            published = tag["content"].strip()[:100]
            break
    main = soup.find("main") or soup.find("article") or soup
    headings = [h.get_text(" ", strip=True)[:300] for h in main.find_all(["h1", "h2", "h3"])][:30]
    links = []
    for anchor in main.find_all("a", href=True)[:50]:
        text = anchor.get_text(" ", strip=True)[:200]
        href = anchor["href"][:2000]
        if text and href.startswith("http"):
            links.append({"text": text, "url": href})
    for tag in soup(["script", "style", "nav", "header", "footer"]):
        tag.decompose()
    text = soup.get_text(" ", strip=True)
    return title, description, published, headings, links, text


def _page_markdown(html):
    """Convert HTML to a compact markdown-ish representation."""
    soup = BeautifulSoup(html, "html.parser")
    for tag in soup(["script", "style", "nav", "header", "footer"]):
        tag.decompose()
    root = soup.find("main") or soup.find("article") or soup.body or soup
    lines = []
    for element in root.descendants:
        name = getattr(element, "name", None)
        if name in ("h1", "h2", "h3", "h4"):
            text = element.get_text(" ", strip=True)
            if text:
                lines.append("#" * min(int(name[1]), 4) + " " + text[:500])
        elif name == "li":
            text = element.get_text(" ", strip=True)
            if text and (not lines or lines[-1] != "- " + text[:500]):
                lines.append("- " + text[:500])
        elif name in ("p", "pre", "blockquote", "td", "th"):
            text = element.get_text(" ", strip=True)
            if text:
                lines.append(text[:2000])
        elif name == "a" and element.get("href", "").startswith("http"):
            text = element.get_text(" ", strip=True)
            if text:
                lines.append(f"[{text[:200]}]({element['href'][:2000]})")
    # Drop duplicates while preserving order (nested elements repeat text).
    seen, compact = set(), []
    for line in lines:
        if line not in seen:
            seen.add(line)
            compact.append(line)
    return "\n\n".join(compact)


def _extract_pdf_text(raw: bytes, max_pages: int = 100):
    import io

    from pypdf import PdfReader

    reader = PdfReader(io.BytesIO(raw))
    pages = min(max(1, max_pages), 100, len(reader.pages))
    parts = [(reader.pages[index].extract_text() or "") for index in range(pages)]
    return "\n".join(parts), len(reader.pages)


async def _search_brave(query, count, key, freshness="any"):
    if not key:
        raise ValueError("Brave Search API Keyが未設定です")
    params = {"q": query, "count": count}
    if freshness in BRAVE_FRESHNESS:
        params["freshness"] = BRAVE_FRESHNESS[freshness]
    async with httpx.AsyncClient(timeout=20, trust_env=False) as client:
        response = await client.get("https://api.search.brave.com/res/v1/web/search", params=params, headers={"X-Subscription-Token": key})
        response.raise_for_status()
        rows = response.json().get("web", {}).get("results", [])
        return {"results": [{"title": row.get("title", ""), "url": row.get("url", ""), "description": row.get("description", "")} for row in rows]}


async def _search_tavily(query, count, key, freshness="any"):
    if not key:
        raise ValueError("Tavily API Keyが未設定です")
    payload = {"api_key": key, "query": query, "max_results": count, "search_depth": "basic"}
    if freshness in TAVILY_TIME_RANGE:
        payload["time_range"] = TAVILY_TIME_RANGE[freshness]
    async with httpx.AsyncClient(timeout=20, trust_env=False) as client:
        response = await client.post("https://api.tavily.com/search", json=payload)
        response.raise_for_status()
        rows = response.json().get("results", [])
        return {"results": [{"title": row.get("title", ""), "url": row.get("url", ""), "description": row.get("content", "")} for row in rows]}


async def _search_exa(query, count, key):
    if not key:
        raise ValueError("Exa API Keyが未設定です")
    async with httpx.AsyncClient(timeout=20, trust_env=False) as client:
        response = await client.post("https://api.exa.ai/search", json={"query": query, "numResults": count, "type": "auto"}, headers={"x-api-key": key, "Content-Type": "application/json"})
        response.raise_for_status()
        rows = response.json().get("results", [])
        return {"results": [{"title": row.get("title", ""), "url": row.get("url", ""), "description": row.get("text", "")} for row in rows]}


async def _search_serper(query, count, key):
    if not key:
        raise ValueError("Serper API Keyが未設定です")
    async with httpx.AsyncClient(timeout=20, trust_env=False) as client:
        response = await client.post("https://google.serper.dev/search", json={"q": query, "num": count}, headers={"X-API-KEY": key, "Content-Type": "application/json"})
        response.raise_for_status()
        rows = response.json().get("organic", [])
        return {"results": [{"title": row.get("title", ""), "url": row.get("link", ""), "description": row.get("snippet", "")} for row in rows]}


async def _search_searxng(query, count, base_url):
    base = (base_url or "").strip().rstrip("/")
    if not base:
        raise ValueError("SearXNGのURLが未設定です")
    parsed = urlsplit(base)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.port not in (None, 443):
        raise ValueError("SearXNGのURLは公開HTTPSエンドポイントを指定してください")
    async with httpx.AsyncClient(timeout=20, trust_env=False) as client:
        response = await client.get(base + "/search", params={"q": query, "format": "json"})
        response.raise_for_status()
        rows = response.json().get("results", [])[:count]
        return {"results": [{"title": row.get("title", ""), "url": row.get("url", ""), "description": row.get("content", "")} for row in rows]}


async def _search_ddgs(query, count, freshness="any"):
    from ddgs import DDGS
    timelimit = DDGS_TIMELIMIT.get(freshness)
    rows = await asyncio.wait_for(asyncio.to_thread(lambda: list(DDGS().text(query, max_results=count, timelimit=timelimit))), timeout=20)
    return {"results": [{"title": row.get("title", ""), "url": row.get("href", ""), "description": row.get("body", "")} for row in rows]}


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


def save_artifact(db, owner, content, name, mime="text/plain", kind="user-created"):
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
            # Lifecycle kind: user-upload / user-created / context-tool-output / context-image.
            # Context-internal artifacts are pinned to their conversation (see
            # _conversation_artifact_ids) instead of having an independent TTL.
            "kind": kind,
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
                status = await runner_request("browser-provisioner", "/status", {}, timeout=10)
            except Exception as error:
                raise ValueError(
                    "Browser導入サービスに接続できません。Docker Composeが起動しているか、"
                    "ネットワーク設定を確認してください。"
                ) from error
            install_status = status.get("status", "not_installed")
            if install_status == "failed":
                raise ValueError("Browserの導入に失敗しました。設定画面から再試行してください。")
            if install_status != "ready":
                raise ValueError(
                    f"Browserはまだ利用できません（現在の状態: {install_status}）。"
                    "設定画面から導入を完了してください。"
                )
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
        title, description, published, headings, links, text = _page_text(html)
        if args.get("format") == "markdown":
            text = _page_markdown(html)
        offset = max(0, min(1000000, int(args.get("offset", 0) or 0)))
        max_chars = max(500, min(30000, int(args.get("max_chars", 8000) or 8000)))
        total = len(text)
        result = {
            "url": args["url"],
            "title": title,
            "description": description,
            "published": published,
            "headings": headings[:10],
            "links": links[:20],
            "text": text[offset:offset + max_chars],
            "chars": total,
            "offset": offset,
            "truncated": offset + max_chars < total,
        }
    elif name == "web_fetch_pdf":
        raw, content_type = await fetch_public_bytes(args["url"], max_bytes=20 * 1024 * 1024)
        if "html" in content_type.lower() and not raw.lstrip().startswith(b"%PDF"):
            raise ValueError("URLはPDFではありません（HTMLが返されました）")
        max_pages = max(1, min(100, int(args.get("max_pages", 100) or 100)))
        try:
            text, pages = await asyncio.wait_for(asyncio.to_thread(_extract_pdf_text, raw, max_pages), timeout=60)
        except Exception as error:
            raise ValueError("PDFの解析に失敗しました: " + type(error).__name__) from error
        result = {"url": args["url"], "pages": pages, "extracted_pages": min(pages, max_pages), "text": text[:50000]}
    elif name == "web_search":
        setting = db.scalar(select(Settings).where(Settings.owner_id == run.owner_id))
        data = setting.data if setting else {}
        count = max(1, min(20, int(args.get("count") or data.get("web_search_count", 5))))
        backend = data.get("web_search_backend", "ddgs")
        freshness = args.get("freshness", "any") or "any"
        query = _apply_domains(args["query"], args.get("domains"))
        if backend == "brave":
            result = await _search_brave(query, count, read_secret(db, data.get("brave_secret_id")), freshness)
        elif backend == "tavily":
            result = await _search_tavily(query, count, read_secret(db, data.get("tavily_secret_id")), freshness)
        elif backend == "exa":
            result = await _search_exa(query, count, read_secret(db, data.get("exa_secret_id")))
        elif backend == "serper":
            result = await _search_serper(query, count, read_secret(db, data.get("serper_secret_id")))
        elif backend == "searxng":
            result = await _search_searxng(query, count, data.get("searxng_url", ""))
        else:
            result = await _search_ddgs(query, count, freshness)
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
    elif name == "knowledge_search":
        result = {"documents": knowledge.search(db, run.owner_id, args.get("query", ""), args.get("top_k", 5))}
    elif name == "knowledge_delete":
        result = knowledge.delete(db, run.owner_id, args["id"])
    elif name == "knowledge_add":
        source_type = args.get("source_type", "text")
        if source_type == "workspace":
            path = args.get("path", "")
            if not path:
                raise ValueError("Workspace path is required")
            fetched = await runner_request("execution", "/execute", {"name": "read_file", "arguments": {"path": path}, "run_id": run.id})
            result = knowledge.add(db, run.owner_id, fetched.get("text", ""), title=args.get("title", "") or path, source_type="workspace", source_ref=path, memo=args.get("memo", ""))
        elif source_type == "url":
            url = args.get("url", "")
            if not url:
                raise ValueError("URL is required")
            raw, content_type = await fetch_public_bytes(url)
            if "pdf" in content_type.lower() or raw.lstrip().startswith(b"%PDF"):
                text, _ = await asyncio.wait_for(asyncio.to_thread(_extract_pdf_text, raw, 100), timeout=60)
                title = args.get("title", "") or url
            else:
                title, _, _, _, _, text = _page_text(raw.decode("utf-8", errors="replace"))
                title = args.get("title", "") or title or url
            result = knowledge.add(db, run.owner_id, text, title=title, source_type="url", source_ref=url, memo=args.get("memo", ""))
        else:
            content = args.get("content", "")
            if not content:
                raise ValueError("Content is required")
            result = knowledge.add(db, run.owner_id, content, title=args.get("title", ""), source_type="text", source_ref="", memo=args.get("memo", ""))
    elif name == "web_clip_save":
        url = args["url"]
        raw, content_type = await fetch_public_bytes(url)
        if "pdf" in content_type.lower() or raw.lstrip().startswith(b"%PDF"):
            text, _ = await asyncio.wait_for(asyncio.to_thread(_extract_pdf_text, raw, 100), timeout=60)
            title = args.get("title", "") or url
        else:
            title, _, _, _, _, text = _page_text(raw.decode("utf-8", errors="replace"))
            title = args.get("title", "") or title or url
        result = knowledge.add(db, run.owner_id, text, title=title, source_type="url", source_ref=url, memo=args.get("memo", ""))
        result = {"saved": True, "url": url, **result}
    elif name == "create_artifact":
        result = {"artifact": save_text_artifact(db, run.owner_id, args["name"], args["mime"], args["content"])}
    else:
        raise ValueError("Unknown tool")
    raw = json.dumps(result, ensure_ascii=False)
    inline_limit = 16000
    try:
        settings_row = db.get(Settings, "settings")
        if settings_row and isinstance(settings_row.data.get("tool_output_inline_limit"), int):
            inline_limit = max(1000, min(100000, settings_row.data["tool_output_inline_limit"]))
    except Exception:
        pass
    if len(raw) > inline_limit:
        artifact = save_artifact(db, run.owner_id, raw.encode(), "tool-result.json", "application/json", kind="context-tool-output")
        try:
            existing_refs = list(run.data.get("tool_refs") or [])
        except Exception:
            existing_refs = []
        if artifact["artifact_id"] not in existing_refs:
            run.data = {**run.data, "tool_refs": [*existing_refs, artifact["artifact_id"]]}
        return {"excerpt": raw[:inline_limit], "truncated": True, "artifact": artifact, "artifact_ref": "artifact://" + artifact["artifact_id"]}
    return result
