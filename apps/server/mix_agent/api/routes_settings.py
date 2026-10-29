import json
import re
from datetime import UTC, datetime, timedelta
from urllib.parse import urlsplit

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import func, select

from mix_agent import config
from mix_agent.api.common import (
    audit,
    context,
    own,
    public,
)
from mix_agent.api.schemas import (
    ConversationFolderInput,
    DiagnosticsView,
    PermissionInput,
    SettingsInput,
    ToolView,
)
from mix_agent.auth.security import (
    store_secret,
)
from mix_agent.db.models import (
    Agent,
    Conversation,
    ConversationFolder,
    Model,
    Permission,
    Run,
    Settings,
)
from mix_agent.memory import service as memory
from mix_agent.tools.execute import runner_request
from mix_agent.tools.registry import call_scope, fingerprint, registry

router = APIRouter(prefix="/api/v1")

# Backward-compatible re-exports for external callers (engine.py, scheduler, etc.)

# Default values for Settings fields. Applied at read-time so new fields are

DEFAULT_SETTINGS = {
    "auto_retry_count": 3,
    "auto_dynamic_switching": True,
    "auto_priority": "balanced",
    "browser_enabled": True,
    "browser_install_requested": False,
    "browser_install_status": "not_installed",
    "browser_install_failure": None,
    "browser_install_progress": None,
    "browser_install_stage": None,
    "browser_timeout_ms": 15000,
    "browser_locale": "ja-JP",
    "browser_user_agent": "",
    "browser_viewport_width": 1280,
    "browser_viewport_height": 720,
    "browser_block_images": False,
    "ui_language": "ja",
    "web_search_enabled": True,
    "web_search_backend": "ddgs",
    "web_search_count": 5,
    "searxng_url": "",
    "tool_settings": {},
    "memory_auto_formation": True,
    "memory_seed_limit": 24,
    "memory_result_limit": 8,
    "memory_link_embedding_neighbors": False,
}


@router.get("/settings")
async def settings(ctx=Depends(context)):
    db, s = ctx
    row = own(db, Settings, "settings", s.owner_id)
    view = public(row)
    # Older profiles predate these preferences; expose compatible defaults.
    view["data"] = {
        **DEFAULT_SETTINGS,
        **{key: value for key, value in view["data"].items() if key not in memory.AUTOMATIC_SETTING_KEYS},
    }
    if view["data"].get("browser_install_requested"):
        state = await browser_install_state(db, row)
        view["data"] = {
            **view["data"],
            "browser_install_status": state["status"],
            "browser_install_failure": state["failure"],
            "browser_install_progress": state["progress"],
            "browser_install_stage": state["stage"],
        }
    return view


async def browser_install_state(db, settings):
    """Read the provisioner state without making the settings page depend on Docker."""
    try:
        state = await runner_request("browser-provisioner", "/status", {}, timeout=5)
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        return {
            "status": settings.data.get("browser_install_status", "not_installed"),
            "failure": settings.data.get("browser_install_failure"),
            "progress": settings.data.get("browser_install_progress"),
            "stage": settings.data.get("browser_install_stage"),
        }
    status = state.get("status", "not_installed")
    failure = state.get("failure")
    progress = state.get("progress")
    stage = state.get("stage")
    if any(
        settings.data.get(key) != value
        for key, value in (
            ("browser_install_status", status),
            ("browser_install_failure", failure),
            ("browser_install_progress", progress),
            ("browser_install_stage", stage),
        )
    ):
        settings.data = {
            **settings.data,
            "browser_install_status": status,
            "browser_install_failure": failure,
            "browser_install_progress": progress,
            "browser_install_stage": stage,
        }
        db.commit()
    return {"status": status, "failure": failure, "progress": progress, "stage": stage}


@router.get("/browser/status")
async def browser_status(ctx=Depends(context)):
    db, s = ctx
    settings = own(db, Settings, "settings", s.owner_id)
    return await browser_install_state(db, settings)


@router.post("/browser/install")
async def browser_install(ctx=Depends(context)):
    db, s = ctx
    settings = own(db, Settings, "settings", s.owner_id)
    try:
        state = await runner_request("browser-provisioner", "/install", {}, timeout=5)
    except Exception as error:
        raise HTTPException(
            503, "Browser導入サービスに接続できません。Docker deploymentを起動してください。"
        ) from error
    settings.data = {
        **settings.data,
        "browser_install_requested": True,
        "browser_install_status": state.get("status", "installing"),
        "browser_install_failure": state.get("failure"),
        "browser_install_progress": state.get("progress"),
        "browser_install_stage": state.get("stage"),
    }
    db.commit()
    return {
        "status": settings.data["browser_install_status"],
        "failure": settings.data["browser_install_failure"],
        "progress": settings.data["browser_install_progress"],
        "stage": settings.data["browser_install_stage"],
    }


@router.post("/browser/update")
async def browser_update(ctx=Depends(context)):
    db, s = ctx
    settings = own(db, Settings, "settings", s.owner_id)
    try:
        state = await runner_request("browser-provisioner", "/update", {}, timeout=5)
    except Exception as error:
        raise HTTPException(
            503, "Browser更新サービスに接続できません。Docker deploymentを起動してください。"
        ) from error
    settings.data = {
        **settings.data,
        "browser_install_requested": True,
        "browser_install_status": state.get("status", "installing"),
        "browser_install_failure": state.get("failure"),
        "browser_install_progress": state.get("progress"),
        "browser_install_stage": state.get("stage"),
    }
    db.commit()
    return state


@router.put("/settings")
def set_settings(body: SettingsInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, Settings, "settings", s.owner_id)
    domains = [
        d.lower().strip()
        for d in (
            body.allowed_domains
            if "allowed_domains" in body.model_fields_set
            else row.data.get("allowed_domains", [])
        )
    ]
    if any(not re.fullmatch(r"[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?", d) or "." not in d for d in domains):
        raise HTTPException(422, "通信許可先は正確なドメイン名で指定してください")
    update_values = body.model_dump(
        exclude={"brave_api_key", "tavily_api_key", "exa_api_key", "serper_api_key", "auto_model_ids"},
        exclude_unset=True,
    )
    data = {**row.data, **update_values, "allowed_domains": domains}
    for key in memory.AUTOMATIC_SETTING_KEYS:
        data.pop(key, None)
    if "searxng_url" in body.model_fields_set and body.searxng_url is not None:
        url = body.searxng_url.strip()
        if url:
            parsed = urlsplit(url)
            if (
                parsed.scheme != "https"
                or not parsed.hostname
                or parsed.username
                or parsed.password
                or parsed.port not in (None, 443)
            ):
                raise HTTPException(422, "SearXNGのURLは公開HTTPSエンドポイントを指定してください")
            if len(url) > 500:
                raise HTTPException(422, "SearXNGのURLが長すぎます")
            data["searxng_url"] = url.rstrip("/")
        else:
            data["searxng_url"] = ""
    if (
        "default_model_id" in body.model_fields_set
        and body.default_model_id
        and body.default_model_id != "auto"
    ):
        own(db, Model, body.default_model_id, s.owner_id)
    if body.auto_model_ids is not None:
        auto_ids = list(dict.fromkeys(body.auto_model_ids))
        for model_id in auto_ids:
            own(db, Model, model_id, s.owner_id)
        data["auto_model_ids"] = auto_ids
    if body.brave_api_key is not None:
        data["brave_secret_id"] = (
            store_secret(db, s.owner_id, body.brave_api_key, "brave") if body.brave_api_key else None
        )
    for field, secret_key, purpose in (
        ("tavily_api_key", "tavily_secret_id", "tavily"),
        ("exa_api_key", "exa_secret_id", "exa"),
        ("serper_api_key", "serper_secret_id", "serper"),
    ):
        value = getattr(body, field)
        if value is not None:
            data[secret_key] = store_secret(db, s.owner_id, value, purpose) if value else None
    row.data = data
    policy_dir = config.DATA / "egress"
    policy_dir.mkdir(exist_ok=True)
    temp = policy_dir / "policy.tmp"
    temp.write_text(json.dumps({"allowed_domains": domains}))
    temp.replace(policy_dir / "policy.json")
    audit(db, s.owner_id, "settings.update", row.id)
    db.commit()
    return public(row)


@router.post("/browser/enable")
def enable_browser_tools(ctx=Depends(context)):
    """Apply the explicit setup choice as one auditable Browser permission bundle."""
    db, s = ctx
    settings = own(db, Settings, "settings", s.owner_id)
    tools = registry(db, s.owner_id)
    browser_ids = [tool_id for tool_id in tools if tool_id.startswith("browser_")]
    values = {**settings.data.get("tool_settings", {})}
    existing = list(db.scalars(select(Permission).where(Permission.owner_id == s.owner_id)))
    for tool_id in browser_ids:
        values[tool_id] = {**values.get(tool_id, {}), "enabled": True}
        for old in existing:
            if old.data.get("agent_id", "") == "" and old.data.get("tool_id") == tool_id:
                db.delete(old)
        tool = tools[tool_id]
        db.add(
            Permission(
                owner_id=s.owner_id,
                data={
                    "agent_id": "",
                    "tool_id": tool_id,
                    "permission": "allow",
                    "tool_version": fingerprint(tool),
                    "scope": call_scope(tool, {}),
                },
            )
        )
    settings.data = {**settings.data, "browser_enabled": True, "tool_settings": values}
    audit(db, s.owner_id, "browser.enable", settings.id)
    db.commit()
    return {"tool_ids": browser_ids}


@router.get("/tools", response_model=list[ToolView])
def tools_list(ctx=Depends(context)):
    db, s = ctx
    # Report the permission the tool actually falls back to, so the Tools screen
    # never shows "always allow" for a destructive tool that will ask.
    return list(registry(db, s.owner_id).values())


@router.post("/permission-rules")
def set_permission(body: PermissionInput, ctx=Depends(context)):
    db, s = ctx
    tool = registry(db, s.owner_id).get(body.tool_id)
    if not tool:
        raise HTTPException(404)
    if body.agent_id:
        own(db, Agent, body.agent_id, s.owner_id)
    data = {
        **body.model_dump(),
        "tool_version": fingerprint(tool),
        "scope": call_scope(tool, {}),
    }
    for old in db.scalars(select(Permission).where(Permission.owner_id == s.owner_id)):
        if old.data.get("agent_id", "") == body.agent_id and old.data.get("tool_id") == body.tool_id:
            db.delete(old)
    row = Permission(owner_id=s.owner_id, data=data)
    db.add(row)
    audit(db, s.owner_id, "permission.update", body.tool_id)
    db.commit()
    return public(row)


@router.get("/conversation-folders")
def list_conversation_folders(ctx=Depends(context)):
    db, s = ctx
    return [
        public(row)
        for row in db.scalars(
            select(ConversationFolder)
            .where(ConversationFolder.owner_id == s.owner_id)
            .order_by(ConversationFolder.data["name"].as_string())
        )
    ]


@router.post("/conversation-folders")
def create_conversation_folder(body: ConversationFolderInput, ctx=Depends(context)):
    db, s = ctx
    row = ConversationFolder(owner_id=s.owner_id, data=body.model_dump())
    db.add(row)
    db.flush()
    audit(db, s.owner_id, "conversation-folder.create", row.id)
    db.commit()
    return public(row)


@router.patch("/conversation-folders/{key}")
def edit_conversation_folder(key: str, body: ConversationFolderInput, ctx=Depends(context)):
    db, s = ctx
    row = own(db, ConversationFolder, key, s.owner_id)
    row.data = body.model_dump()
    audit(db, s.owner_id, "conversation-folder.update", key)
    db.commit()
    return public(row)


@router.delete("/conversation-folders/{key}")
def delete_conversation_folder(key: str, ctx=Depends(context)):
    db, s = ctx
    row = own(db, ConversationFolder, key, s.owner_id)
    for conversation in db.scalars(
        select(Conversation).where(
            Conversation.owner_id == s.owner_id,
            Conversation.data["folder_id"].astext == key,
        )
    ):
        conversation.data = {**conversation.data, "folder_id": None}
    db.delete(row)
    audit(db, s.owner_id, "conversation-folder.delete", key)
    db.commit()
    return {"ok": True}


@router.get("/settings/diagnostics", response_model=DiagnosticsView)
def settings_diagnostics(ctx=Depends(context)):
    """Return a privacy-minimal operational snapshot for the signed-in owner."""
    db, session = ctx
    owner_id = session.owner_id
    checked_at = datetime.now(UTC)

    # Query only aggregate counts and timestamps. Conversation text, Tool output,
    # provider errors, and credentials never leave the server in this response.
    run_counts = dict(
        db.execute(
            select(Run.status, func.count()).where(Run.owner_id == owner_id).group_by(Run.status)
        ).all()
    )
    recent_runs = db.scalars(
        select(Run)
        .where(Run.owner_id == owner_id, Run.status.in_(["failed", "interrupted"]))
        .order_by(Run.created_at.desc())
        .limit(10)
    )
    failures = [
        {
            "status": run.status,
            "mode": run.data.get("mode", "unknown"),
            "created_at": run.created_at.isoformat(),
        }
        for run in recent_runs
    ]
    db.execute(select(1))

    runner_status = {
        "database": "ok",
        "execution": "configured" if config.runner_token("execution") else "not_configured",
        "mcp": "configured" if config.runner_token("mcp") else "not_configured",
        "browser": "configured" if config.runner_token("browser") else "not_configured",
        "mcp_manager": "configured" if config.runner_token("manager") else "not_configured",
    }
    settings_row = db.scalar(select(Settings).where(Settings.owner_id == owner_id))
    if settings_row and settings_row.data.get("browser_install_requested"):
        runner_status["browser_install"] = settings_row.data.get("browser_install_status", "unknown")

    now_utc = checked_at
    stale_runs = (
        db.scalar(
            select(func.count())
            .select_from(Run)
            .where(
                Run.owner_id == owner_id,
                Run.status.in_(["queued", "running"]),
                Run.created_at < now_utc - timedelta(minutes=30),
            )
        )
        or 0
    )
    return {
        "checked_at": checked_at.isoformat(),
        "services": runner_status,
        "runs": {
            "active": sum(
                run_counts.get(status, 0) for status in ("queued", "running", "waiting_approval", "paused")
            ),
            "failed_30d": db.scalar(
                select(func.count())
                .select_from(Run)
                .where(
                    Run.owner_id == owner_id,
                    Run.status == "failed",
                    Run.created_at >= now_utc - timedelta(days=30),
                )
            )
            or 0,
            "interrupted_30d": db.scalar(
                select(func.count())
                .select_from(Run)
                .where(
                    Run.owner_id == owner_id,
                    Run.status == "interrupted",
                    Run.created_at >= now_utc - timedelta(days=30),
                )
            )
            or 0,
            "stale_active": stale_runs,
            "recent_failures": failures,
        },
    }
