from fastapi import APIRouter, Depends, HTTPException, Request
from sqlalchemy import select

from mix_agent import pricing
from mix_agent.api.common import (
    audit,
    context,
    own,
    public,
    set_title_placeholder,
)
from mix_agent.api.schemas import (
    AgentInput,
    ConversationInput,
    Input,
    ModelInput,
    PermissionRuleView,
    ProjectInput,
    RecordView,
)
from mix_agent.db.models import (
    Agent,
    Conversation,
    Memory,
    Model,
    Permission,
    Project,
    Provider,
    Run,
    Skill,
    now,
)
from mix_agent.memory import service as memory
from mix_agent.providers.reasoning import reasoning_control
from mix_agent.runs.state import NON_TERMINAL_STATES
from mix_agent.skills import service as skills
from mix_agent.tools.registry import registry

router = APIRouter(prefix="/api/v1")


COLLECTIONS = {
    "models": (Model, ModelInput),
    "agents": (Agent, AgentInput),
    "conversations": (Conversation, ConversationInput),
    "projects": (Project, ProjectInput),
    "permission-rules": (Permission, None),
}


async def list_collection(collection: str, ctx=Depends(context)):
    db, s = ctx
    if collection not in COLLECTIONS:
        raise HTTPException(404)
    cls, _ = COLLECTIONS[collection]
    rows = [
        public(r)
        for r in db.scalars(select(cls).where(cls.owner_id == s.owner_id).order_by(cls.created_at.desc()))
    ]
    if collection == "models":
        providers = {
            p.id: p.data for p in db.scalars(select(Provider).where(Provider.owner_id == s.owner_id))
        }
        for row in rows:
            data = row["data"]
            provider = providers.get(data["provider_id"], {})
            data["reasoning_control"] = reasoning_control(provider.get("kind", ""), data["model_id"])
            data["pricing"], data["pricing_source"] = pricing.resolve(data)
    return rows


def validate_entity(collection, value, db, owner):
    if collection == "models":
        own(db, Provider, value["provider_id"], owner)
    elif collection == "agents":
        if value["model_id"] and value["model_id"] != "auto":
            own(db, Model, value["model_id"], owner)
        if any(t not in registry(db, owner) for t in value["tool_ids"]):
            raise HTTPException(422, "未登録Toolがあります")
        if any(
            not db.get(Skill, skill_id) or db.get(Skill, skill_id).owner_id != owner
            for skill_id in value["skill_ids"]
        ):
            raise HTTPException(422, "未登録Skillがあります")
    elif collection == "conversations" and value.get("project_id"):
        own(db, Project, value["project_id"], owner)
    elif collection == "projects" and (
        any(len(source) > 20000 for source in value["sources"]) or sum(map(len, value["sources"])) > 50000
    ):
        raise HTTPException(422, "資料テキストは合計5万文字以内にしてください")


def apply_context_override(value, previous=None):
    """Set the effective context limit while retaining provider metadata."""
    if value.get("context_window_override") is not None:
        value["context_window"] = value["context_window_override"]
        value["context_source"] = "manual"
        value["context_confidence"] = "runtime"
        value["metadata"] = {
            **(previous or {}).get("metadata", {}),
            "context_window": {
                "value": value["context_window_override"],
                "source": "manual",
                "confidence": "runtime",
                "resolved_at": now().isoformat(),
            },
        }
        return value
    if previous and previous.get("provider_context_window"):
        value["context_window"] = previous["provider_context_window"]
        value["context_source"] = previous.get("provider_context_source") or "provider_api"
        evidence = previous.get("metadata", {}).get("context_window")
        if evidence:
            value["context_confidence"] = evidence.get("confidence")
    return value


async def create_entity(collection: str, request: Request, ctx=Depends(context)):
    db, s = ctx
    if collection not in COLLECTIONS or COLLECTIONS[collection][1] is None:
        raise HTTPException(404)
    cls, schema = COLLECTIONS[collection]
    from pydantic import ValidationError

    try:
        payload = await request.json()
        value = schema.model_validate(payload).model_dump()
    except ValidationError:
        raise HTTPException(422, "入力項目を確認してください")
    if collection == "conversations":
        set_title_placeholder(value, explicit="title" in payload)
    validate_entity(collection, value, db, s.owner_id)
    if collection == "models":
        value = apply_context_override(value)
    row = cls(owner_id=s.owner_id, data=value)
    db.add(row)
    db.commit()
    return public(row)


async def edit_entity(collection: str, key: str, request: Request, ctx=Depends(context)):
    db, s = ctx
    if collection not in COLLECTIONS or COLLECTIONS[collection][1] is None:
        raise HTTPException(404)
    cls, schema = COLLECTIONS[collection]
    row = own(db, cls, key, s.owner_id)
    from pydantic import ValidationError

    try:
        payload = await request.json()
        value = schema.model_validate(
            {**{k: v for k, v in row.data.items() if k in schema.model_fields}, **payload}
        ).model_dump()
    except ValidationError:
        raise HTTPException(422, "入力項目を確認してください")
    validate_entity(collection, value, db, s.owner_id)
    if collection == "models":
        value = apply_context_override(value, row.data)
        # Omitted pricing keeps the saved override; an explicit null clears it.
        if "pricing_override" not in payload:
            value["pricing_override"] = row.data.get("pricing_override")
    if collection == "conversations" and "title" in payload:
        set_title_placeholder(value, explicit=True)
    # Probe results are system-managed state, not part of the public model
    # editor schema. Keep them when the user edits a model.
    row.data = {**row.data, **value}
    db.commit()
    return public(row)


@router.delete("/{collection}/{key}")
def delete_entity(collection: str, key: str, ctx=Depends(context)):
    db, s = ctx
    if collection == "memories":
        own(db, Memory, key, s.owner_id)
        memory.change(db, s.owner_id, memory_id=key, delete=True)
    elif collection == "skills":
        row = own(db, Skill, key, s.owner_id)
        skills.change(db, s.owner_id, skill_id=key, delete=True, enabled=False)
    elif collection in ("providers", "models", "agents", "permission-rules"):
        cls = {"providers": Provider, "models": Model, "agents": Agent, "permission-rules": Permission}[
            collection
        ]
        row = own(db, cls, key, s.owner_id)
        if db.scalar(
            select(Run.id).where(
                Run.owner_id == s.owner_id,
                Run.status.in_(NON_TERMINAL_STATES),
            )
        ):
            raise HTTPException(409, "実行を停止してから削除してください")
        if collection == "providers" and any(
            model.data.get("provider_id") == key
            for model in db.scalars(select(Model).where(Model.owner_id == s.owner_id))
        ):
            raise HTTPException(409, "先に関連モデルを削除してください")
        db.delete(row)
    else:
        raise HTTPException(404)
    audit(db, s.owner_id, "delete." + collection, key)
    db.commit()
    return {"ok": True}


# Concrete routes expose request/response schemas to generated TypeScript clients.
def register_collection(name, schema, view_schema=RecordView, include_list=True):
    async def listing(ctx=Depends(context)):
        return await list_collection(name, ctx)

    if include_list:
        router.add_api_route(
            "/" + name, listing, methods=["GET"], response_model=list[view_schema], name="list_" + name
        )
    if schema is None:
        return

    async def creating(body, request: Request, ctx=Depends(context)):
        return await create_entity(name, request, ctx)

    creating.__annotations__["body"] = schema
    router.add_api_route(
        "/" + name, creating, methods=["POST"], response_model=view_schema, name="create_" + name
    )
    from pydantic import create_model

    partial = create_model(
        schema.__name__ + "Patch",
        __base__=Input,
        **{k: (v.annotation | None, None) for k, v in schema.model_fields.items()},
    )

    async def editing(key: str, body, request: Request, ctx=Depends(context)):
        return await edit_entity(name, key, request, ctx)

    editing.__annotations__["body"] = partial
    router.add_api_route(
        "/" + name + "/{key}", editing, methods=["PATCH"], response_model=view_schema, name="edit_" + name
    )


for collection_name, (_, collection_schema) in COLLECTIONS.items():
    register_collection(
        collection_name,
        collection_schema,
        PermissionRuleView if collection_name == "permission-rules" else RecordView,
        include_list=collection_name != "conversations",
    )
