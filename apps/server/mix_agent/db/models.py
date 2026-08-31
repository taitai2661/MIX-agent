from datetime import datetime, timezone
from uuid import uuid4
from sqlalchemy import String, Text, Integer, DateTime, JSON, ForeignKey, UniqueConstraint, Index
from sqlalchemy.dialects.postgresql import JSONB
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


def uid():
    return str(uuid4())


def now():
    return datetime.now(timezone.utc)


class Base(DeclarativeBase):
    pass


JSONType = JSON().with_variant(JSONB(), "postgresql")


class Record:
    id: Mapped[str] = mapped_column(String(36), primary_key=True, default=uid)
    owner_id: Mapped[str] = mapped_column(String(36), index=True)
    data: Mapped[dict] = mapped_column(JSONType, default=dict)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=now)


class User(Base):
    __tablename__ = "users"
    id: Mapped[str] = mapped_column(String(36), primary_key=True, default=uid)
    username: Mapped[str] = mapped_column(String(100), unique=True)
    password_hash: Mapped[str] = mapped_column(Text)
    singleton: Mapped[int] = mapped_column(Integer, unique=True, default=1)


class Session(Base):
    __tablename__ = "sessions"
    id: Mapped[str] = mapped_column(String(64), primary_key=True)
    owner_id: Mapped[str] = mapped_column(ForeignKey("users.id"))
    csrf: Mapped[str] = mapped_column(String(64))
    expires: Mapped[datetime] = mapped_column(DateTime(timezone=True))


class Provider(Record, Base):
    __tablename__ = "providers"


class Model(Record, Base):
    __tablename__ = "models"


class Secret(Record, Base):
    __tablename__ = "secrets"


class Agent(Record, Base):
    __tablename__ = "agents"


class Workspace(Record, Base):
    __tablename__ = "workspaces"


class Conversation(Record, Base):
    __tablename__ = "conversations"


class Message(Record, Base):
    __tablename__ = "messages"
    conversation_id: Mapped[str] = mapped_column(ForeignKey("conversations.id"), index=True)


class Feedback(Record, Base):
    __tablename__ = "message_feedback"
    message_id: Mapped[str] = mapped_column(ForeignKey("messages.id"), unique=True, index=True)


class Settings(Record, Base):
    __tablename__ = "app_settings"


class Tool(Record, Base):
    __tablename__ = "tools"


class MCPConnection(Record, Base):
    __tablename__ = "mcp_connections"


class Permission(Record, Base):
    __tablename__ = "permission_rules"


class Memory(Record, Base):
    __tablename__ = "memories"


class MemoryRevision(Record, Base):
    __tablename__ = "memory_revisions"


class Skill(Record, Base):
    __tablename__ = "skills"


class SkillRevision(Record, Base):
    __tablename__ = "skill_revisions"


class Artifact(Record, Base):
    __tablename__ = "artifacts"


class Audit(Record, Base):
    __tablename__ = "audit_events"


class Run(Record, Base):
    __tablename__ = "runs"
    conversation_id: Mapped[str] = mapped_column(ForeignKey("conversations.id"), index=True)
    status: Mapped[str] = mapped_column(String(30), default="queued", index=True)
    request_key: Mapped[str] = mapped_column(String(100), unique=True)
    __table_args__ = (
        Index(
            "one_active_run",
            "conversation_id",
            unique=True,
            postgresql_where=status.in_(["queued", "running", "waiting_approval"]),
            sqlite_where=status.in_(["queued", "running", "waiting_approval"]),
        ),
    )


class Event(Base):
    __tablename__ = "run_events"
    id: Mapped[str] = mapped_column(String(36), primary_key=True, default=uid)
    run_id: Mapped[str] = mapped_column(ForeignKey("runs.id"), index=True)
    sequence: Mapped[int] = mapped_column(Integer)
    kind: Mapped[str] = mapped_column(String(50))
    data: Mapped[dict] = mapped_column(JSONType)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=now)
    __table_args__ = (UniqueConstraint("run_id", "sequence"),)


class ToolCall(Record, Base):
    __tablename__ = "tool_calls"
    run_id: Mapped[str] = mapped_column(ForeignKey("runs.id"), index=True)
    status: Mapped[str] = mapped_column(String(30), default="pending")


class Approval(Record, Base):
    __tablename__ = "approvals"
    run_id: Mapped[str] = mapped_column(ForeignKey("runs.id"), index=True)
    tool_call_id: Mapped[str] = mapped_column(ForeignKey("tool_calls.id"), unique=True)
    status: Mapped[str] = mapped_column(String(30), default="pending")
    expires: Mapped[datetime] = mapped_column(DateTime(timezone=True))


ENTITIES = {
    c.__tablename__: c
    for c in (
        Provider,
        Model,
        Agent,
        Conversation,
        MCPConnection,
        Permission,
        Memory,
        Artifact,
        Tool,
        Settings,
        Workspace,
    )
}
