from datetime import UTC, datetime
from uuid import uuid4

from sqlalchemy import (
    Boolean,
    CheckConstraint,
    DateTime,
    ForeignKey,
    Index,
    Integer,
    String,
    Text,
    UniqueConstraint,
)
from sqlalchemy.dialects.postgresql import JSONB
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column


def uid():
    return str(uuid4())


def now():
    return datetime.now(UTC)


class Base(DeclarativeBase):
    pass


JSONType = JSONB


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


class LoginEvent(Base):
    __tablename__ = "login_events"
    id: Mapped[str] = mapped_column(String(36), primary_key=True, default=uid)
    owner_id: Mapped[str] = mapped_column(ForeignKey("users.id"), index=True)
    successful: Mapped[bool] = mapped_column(Boolean)
    ip: Mapped[str] = mapped_column(String(255), default="unknown")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=now, index=True)


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


class Project(Record, Base):
    __tablename__ = "projects"


class ConversationFolder(Record, Base):
    __tablename__ = "conversation_folders"


class Message(Record, Base):
    __tablename__ = "messages"
    conversation_id: Mapped[str] = mapped_column(ForeignKey("conversations.id"), index=True)


class Feedback(Record, Base):
    __tablename__ = "message_feedback"
    message_id: Mapped[str] = mapped_column(ForeignKey("messages.id"), unique=True, index=True)


class AutoReliabilityEvent(Record, Base):
    """A privacy-minimal, time-bounded record of an Auto provider attempt."""
    __tablename__ = "auto_reliability_events"
    __table_args__ = (
        Index("auto_reliability_owner_created", "owner_id", "created_at"),
    )


class PerformanceEvent(Record, Base):
    """A privacy-minimal, time-bounded measurement of a completed answer."""
    __tablename__ = "performance_events"
    __table_args__ = (
        Index("performance_event_owner_created", "owner_id", "created_at"),
    )


class ModelUsageEvent(Record, Base):
    """A privacy-minimal, time-bounded record of one model call's token usage."""
    __tablename__ = "model_usage_events"
    __table_args__ = (
        Index("model_usage_owner_created", "owner_id", "created_at"),
    )


class ScheduledJob(Record, Base):
    __tablename__ = "scheduled_jobs"
    __table_args__ = (Index("scheduled_job_owner_enabled", "owner_id", "created_at"),)


class ScheduledRun(Record, Base):
    __tablename__ = "scheduled_runs"
    job_id: Mapped[str] = mapped_column(ForeignKey("scheduled_jobs.id"), index=True)
    scheduled_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), index=True)
    status: Mapped[str] = mapped_column(String(30), default="pending", index=True)
    run_id: Mapped[str | None] = mapped_column(ForeignKey("runs.id"), nullable=True, index=True)
    __table_args__ = (UniqueConstraint("job_id", "scheduled_at"),)


class Notification(Record, Base):
    __tablename__ = "notifications"
    read_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True, index=True)
    __table_args__ = (Index("notification_owner_read", "owner_id", "read_at"),)


class PushSubscription(Record, Base):
    __tablename__ = "push_subscriptions"
    endpoint: Mapped[str] = mapped_column(Text, unique=True)
    cursor_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=now)
    cursor_id: Mapped[str] = mapped_column(String(36), default="")


class Settings(Record, Base):
    __tablename__ = "app_settings"


class Tool(Record, Base):
    __tablename__ = "tools"


class MCPConnection(Record, Base):
    __tablename__ = "mcp_connections"


class MCPAuthState(Record, Base):
    __tablename__ = "mcp_auth_states"
    state_hash: Mapped[str] = mapped_column(String(64), default="", index=True)
    __table_args__ = (Index("mcp_auth_state_owner_created", "owner_id", "created_at"),)


class Permission(Record, Base):
    __tablename__ = "permission_rules"


class Memory(Record, Base):
    """A single ChatGPT-style Memory item.

    ChatGPT-like flat structure: one short user fact/preference per row.
    ``content`` is first-class (not buried in JSONB) so list/search/edit
    stays trivial. ``data`` keeps only UI hints: ``pinned`` / ``source``
    (``user`` explicit vs ``auto`` extracted) / ``source_run``.
    All role/scope/lifecycle/verification/association machinery was removed
    in 0022 (see migration) — retrieval is plain text match + recency.
    """

    __tablename__ = "memories"
    content: Mapped[str] = mapped_column(Text, default="", nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=now)
    __table_args__ = (
        Index("memory_owner_updated", "owner_id", "updated_at"),
    )


# NOTE: MemoryAssociation / MemoryFeature / MemoryConflict / MemoryEvidence
# were removed in the ChatGPT-style simplification (migration 0022 drops the
# tables). Do not re-add them; use plain Memory rows only.


class MemoryProcessingJob(Record, Base):
    __tablename__ = "memory_processing_jobs"
    run_id: Mapped[str] = mapped_column(ForeignKey("runs.id"), index=True)
    status: Mapped[str] = mapped_column(String(20), default="pending", index=True)
    attempts: Mapped[int] = mapped_column(Integer, default=0)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=now)
    __table_args__ = (UniqueConstraint("run_id"), Index("memory_job_status_created", "status", "created_at"))


class MemoryActionEvent(Record, Base):
    __tablename__ = "memory_action_events"
    run_id: Mapped[str | None] = mapped_column(ForeignKey("runs.id"), nullable=True, index=True)
    candidate_index: Mapped[int] = mapped_column(Integer, default=0)
    action_version: Mapped[int] = mapped_column(Integer, default=1)
    __table_args__ = (
        UniqueConstraint("run_id", "candidate_index", "action_version"),
        Index("memory_action_owner_created", "owner_id", "created_at"),
    )


class MemoryRevision(Record, Base):
    __tablename__ = "memory_revisions"


class Skill(Record, Base):
    __tablename__ = "skills"


class SkillRevision(Record, Base):
    __tablename__ = "skill_revisions"


class Knowledge(Record, Base):
    """One chunk of a saved knowledge document. Chunks of a document share data.doc_id."""

    __tablename__ = "knowledge"
    __table_args__ = (Index("knowledge_owner_doc", "owner_id", "created_at"),)


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
            postgresql_where=status.in_([
                "queued", "running", "waiting_approval", "paused", "budget_extension_pending",
            ]),
        ),
        CheckConstraint(
            "status IN ('queued', 'running', 'waiting_approval', 'paused', "
            "'budget_extension_pending', 'completed', 'failed', 'cancelled', 'interrupted')",
            name="ck_runs_status_valid",
        ),
    )


class RunCheckpoint(Record, Base):
    """Durable per-step snapshot of a long-running Run for selective resume.

    A checkpoint freezes the structured state (history, task_state, agent_plan,
    summary, recent tool calls, and bookkeeping counters) at a known step so
    the user can resume from that exact point instead of starting over.
    """

    __tablename__ = "run_checkpoints"
    run_id: Mapped[str] = mapped_column(ForeignKey("runs.id"), index=True)
    step: Mapped[int] = mapped_column(Integer, default=0)
    tool_count: Mapped[int] = mapped_column(Integer, default=0)
    trigger: Mapped[str] = mapped_column(String(30), default="interval")
    __table_args__ = (
        Index("run_checkpoint_run_step", "run_id", "step"),
        Index("run_checkpoint_run_created", "run_id", "created_at"),
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
