from datetime import UTC, datetime
from uuid import uuid4

from sqlalchemy import (
    Boolean,
    CheckConstraint,
    DateTime,
    Float,
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
    __table_args__ = (Index("sessions_expires", "expires"),)


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
    # The composite (conversation_id, created_at) index below covers queries
    # that filter by conversation alone, so the implicit single-column index
    # was pure write/storage overhead.
    conversation_id: Mapped[str] = mapped_column(ForeignKey("conversations.id"))
    __table_args__ = (Index("messages_conversation_created", "conversation_id", "created_at"),)


class Feedback(Record, Base):
    __tablename__ = "message_feedback"
    message_id: Mapped[str] = mapped_column(ForeignKey("messages.id"), unique=True, index=True)


class AutoReliabilityEvent(Record, Base):
    """A privacy-minimal, time-bounded record of an Auto provider attempt."""

    __tablename__ = "auto_reliability_events"
    __table_args__ = (Index("auto_reliability_owner_created", "owner_id", "created_at"),)


class PerformanceEvent(Record, Base):
    """A privacy-minimal, time-bounded measurement of a completed answer."""

    __tablename__ = "performance_events"
    __table_args__ = (Index("performance_event_owner_created", "owner_id", "created_at"),)


class ModelUsageEvent(Record, Base):
    """A privacy-minimal, time-bounded record of one model call's token usage."""

    __tablename__ = "model_usage_events"
    __table_args__ = (Index("model_usage_owner_created", "owner_id", "created_at"),)


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
    """A single ChatGPT-style Memory item: one short user fact per row.

    ``content`` stays first-class (not buried in JSONB) so list/search/edit
    remains trivial, and ``data`` keeps the row-specific extras spec §1.1
    leaves to each edition: ``pinned`` / ``source`` / ``source_run`` /
    ``strength`` / ``activation_count`` / ``embedding``.

    Judgment change (recorded in ``docs/memory-roadmap.md`` §2.2, decision B):
    migration 0022 dropped the whole taxonomy *and* the four graph tables.
    Spec §1 is the contract for both editions, so migration 0024 restores the
    eight columns it needs — ``lifecycle_state``, ``superseded_by_id``,
    ``disputed_by_id``, ``confidence``, ``salience``, ``role``, ``scope``,
    ``source_kind`` — and those eight are now real columns, not constants
    echoed back by ``service._trace_data``.

    What 0022 deleted and is still deliberately *not* back:
    ``title`` / ``verification`` / ``activation_count`` / ``task_id`` as
    columns (the flat UI does not render them; ``activation_count`` rides in
    ``data``), and three of the four graph tables ``MemoryAssociation`` /
    ``MemoryFeature`` / ``MemoryConflict`` / ``MemoryEvidence``.
    Supersession and conflict are expressed *on the row* (§1's
    ``superseded_by_id`` / ``disputed_by_id``); associations get the single
    ``memory_links`` table of spec §4, restored by migration 0025.
    Re-adding a per-concept table is a spec change, not an implementation
    detail — see the NOTE below.
    """

    __tablename__ = "memories"
    content: Mapped[str] = mapped_column(Text, default="", nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=now)

    # --- spec §1, restored by migration 0024 (defaults are spec §1's) --------
    # Validated against ``memory/types.py`` (spec §2) before every write; an
    # unknown value never reaches the column.
    lifecycle_state: Mapped[str] = mapped_column(
        String(20),
        nullable=False,
        default="established",
        server_default="established",
    )
    superseded_by_id: Mapped[str | None] = mapped_column(String(36), nullable=True, default=None)
    disputed_by_id: Mapped[str | None] = mapped_column(String(36), nullable=True, default=None)
    confidence: Mapped[float] = mapped_column(
        Float,
        nullable=False,
        default=0.7,
        server_default="0.7",
    )
    salience: Mapped[float] = mapped_column(
        Float,
        nullable=False,
        default=0.5,
        server_default="0.5",
    )
    role: Mapped[str] = mapped_column(
        String(20),
        nullable=False,
        default="fact",
        server_default="fact",
    )
    scope: Mapped[str] = mapped_column(
        String(20),
        nullable=False,
        default="user",
        server_default="user",
    )
    source_kind: Mapped[str] = mapped_column(
        String(20),
        nullable=False,
        default="agent",
        server_default="agent",
    )
    __table_args__ = (
        Index("memory_owner_updated", "owner_id", "updated_at"),
        Index("ix_memories_lifecycle_state", "lifecycle_state"),
    )


# NOTE: migration 0022 dropped memory_associations / memory_features /
# memory_conflicts / memory_evidences, and 0024 deliberately did *not* bring
# any of them back: supersession and conflict live on the row itself
# (``superseded_by_id`` / ``disputed_by_id``, spec §1/§3) and evidence and
# features stay in the row's ``data``. Migration 0025 then restored *exactly
# one* table — spec §4's ``memory_links`` (decision A = plan B: the flat UI
# keeps its one-row-per-fact shape, associations get the single link table).
# The other three stay deleted: re-creating four per-concept tables would be
# a spec change, not an implementation detail.


class MemoryLink(Base):
    """One directed relation between two memories (spec §4).

    The table is the *shadow* of §3 for the two transition kinds: a
    ``supersedes`` link exists because the target row carries
    ``superseded_by_id`` pointing at the source, and ``contradicts`` exists
    because the target carries ``disputed_by_id`` (§4: "二重管理にしない").
    ``memory.service.sync_links`` is the single writer that keeps the pair
    equal, so neither side can move alone. The other §2 ``relation_kinds``
    (``supports`` / ``derived_from`` / ``caused_by`` / ``relates_to``) have no
    column and live only here.

    ``ondelete="CASCADE"`` keeps §7.2's ``forget`` working: deleting a memory
    drops its links in the same statement instead of blocking on the foreign
    key.
    """

    __tablename__ = "memory_links"

    id: Mapped[str] = mapped_column(String(36), primary_key=True, default=uid)
    owner_id: Mapped[str] = mapped_column(String(36), nullable=False)
    source_memory_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("memories.id", ondelete="CASCADE"),
        nullable=False,
    )
    target_memory_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("memories.id", ondelete="CASCADE"),
        nullable=False,
    )
    kind: Mapped[str] = mapped_column(String(20), nullable=False)
    weight: Mapped[float] = mapped_column(Float, nullable=False, default=0.2, server_default="0.2")
    coactivation_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0, server_default="0")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=now, nullable=False)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=now, nullable=False)
    __table_args__ = (
        # Spec §4: (owner, source, target, kind) is unique — a duplicate upsert
        # updates one row instead of adding a second edge.
        UniqueConstraint(
            "owner_id",
            "source_memory_id",
            "target_memory_id",
            "kind",
            name="uq_memory_links_owner_source_target_kind",
        ),
        Index("memory_link_owner_source", "owner_id", "source_memory_id"),
        Index("memory_link_owner_target", "owner_id", "target_memory_id"),
    )


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
    __table_args__ = (Index("audit_events_owner_created", "owner_id", "created_at"),)


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
            postgresql_where=status.in_(
                [
                    "queued",
                    "running",
                    "waiting_approval",
                    "paused",
                    "budget_extension_pending",
                ]
            ),
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
    __table_args__ = (
        UniqueConstraint("run_id", "sequence"),
        Index("run_events_created", "created_at"),
    )


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
