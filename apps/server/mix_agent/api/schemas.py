from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")


class RecordView(BaseModel):
    id: str
    data: dict
    created_at: str


ChatMode = Literal["chat", "thinking", "agent"]
RunStatus = Literal[
    "queued", "running", "waiting_approval", "paused", "budget_extension_pending",
    "completed", "failed", "cancelled", "interrupted",
]
ApprovalDecision = Literal["once", "always", "denied"]
ApprovalStatus = Literal["pending", "once", "always", "denied", "expired"]


class ArtifactView(BaseModel):
    artifact_id: str
    name: str
    mime: str
    size: int


class AutoSelectionView(BaseModel):
    model_id: str
    model_name: str | None = None
    reason: str
    model_record_id: str | None = None
    profile: str | None = None

    model_config = ConfigDict(extra="allow")


class PerformanceView(BaseModel):
    output_tokens: int
    generation_ms: int
    tokens_per_second: float
    first_output_ms: int | None = None


class MessageDataView(BaseModel):
    role: Literal["user", "assistant"]
    content: str
    artifact_ids: list[str] = Field(default_factory=list)
    artifacts: list[ArtifactView] = Field(default_factory=list)
    auto_selection: AutoSelectionView | None = None
    performance: PerformanceView | None = None
    feedback: Literal["up", "down"] | None = None

    model_config = ConfigDict(extra="allow")


class MessageView(BaseModel):
    id: str
    data: MessageDataView
    created_at: str


class ContextSummaryView(BaseModel):
    text: str
    covered_count: int = 0
    updated_at: str | None = None


class AnswerEvaluationView(BaseModel):
    status: Literal["provided", "needs_review", "unanswered"]
    reason: str


class ConversationRunView(BaseModel):
    id: str
    status: RunStatus
    reason: str | None = None
    message_id: str | None = None
    context_summary: ContextSummaryView | None = None
    answer_evaluation: AnswerEvaluationView | None = None


class ConversationSelectionView(BaseModel):
    model_id: str
    agent_id: str = ""
    mode: ChatMode = "chat"
    research_mode: bool = False


class ConversationMessagesView(BaseModel):
    messages: list[MessageView]
    runs: list[ConversationRunView]
    selection: ConversationSelectionView | None = None
    project_id: str | None = None
    todos: list[dict] = Field(default_factory=list)


class SendMessageView(BaseModel):
    run_id: str


class ToolCallActivityView(BaseModel):
    label: str | None = None
    detail: str | None = None
    sources: list[dict[str, str]] = Field(default_factory=list)
    remaining: int | None = None

    model_config = ConfigDict(extra="allow")


class ToolCallRetryView(BaseModel):
    available: bool
    label: str


class ToolCallApprovalView(BaseModel):
    id: str
    status: ApprovalStatus
    tool: str | None = None
    risk: str | None = None


class ToolCallHistoryView(BaseModel):
    id: str
    run_id: str
    status: Literal["completed", "failed", "running", "waiting_approval", "unknown"]
    tool_name: str
    activity: ToolCallActivityView | None = None
    result_activity: ToolCallActivityView | None = None
    created_at: str
    result: object | None = None
    failure: str | None = None
    artifact: ArtifactView | None = None
    approval: ToolCallApprovalView | None = None
    retry: ToolCallRetryView

    model_config = ConfigDict(extra="allow")


class ApprovalDataView(BaseModel):
    tool: str
    arguments: dict = Field(default_factory=dict)
    scope: dict = Field(default_factory=dict)
    risk: str | None = None
    tool_version: str | None = None

    model_config = ConfigDict(extra="allow")


class ApprovalView(BaseModel):
    id: str
    data: ApprovalDataView
    created_at: str
    status: ApprovalStatus


class RunBudgetView(BaseModel):
    max_seconds: int | None = None
    max_steps: int | None = None
    max_tool_calls: int | None = None


class BudgetExtensionRequestView(BaseModel):
    tool_calls_used: int
    tool_calls_limit: int
    steps_used: int
    steps_limit: int
    elapsed_seconds: int
    max_seconds: int
    requested_at: str


class RunCheckpointView(BaseModel):
    id: str
    step: int
    tool_count: int
    trigger: str
    created_at: str | None = None


class RunView(BaseModel):
    id: str
    status: RunStatus
    browser_manual_active: bool = False
    browser_pause_requested: bool = False
    reason: str | None = None
    steps: int
    tool_count: int
    mode: Literal["chat", "thinking", "agent"]
    policy: dict[str, str | int | bool]
    budget: RunBudgetView
    remaining: RunBudgetView
    approvals: list[ApprovalView]
    context_summary: ContextSummaryView | None = None
    answer_evaluation: AnswerEvaluationView | None = None
    budget_extension_request: BudgetExtensionRequestView | None = None
    budget_extensions_used: int = 0
    budget_extensions_max: int = 0
    stagnation: list[dict] | None = None
    checkpoint_resumed_from: dict | None = None
    checkpoints: list[RunCheckpointView] = Field(default_factory=list)


class StatisticsSummaryView(BaseModel):
    key: str
    total: int
    success: int
    failure: int
    failure_rate: float
    first_output_ms: int | None = None
    completion_ms: int | None = None
    tokens_per_second: float | None = None
    tps_count: int
    classifications: dict[str, int] = Field(default_factory=dict)


class StatisticsGroupView(StatisticsSummaryView):
    model_id: str
    provider_id: str
    scope: str
    model_name: str


class StatisticsView(BaseModel):
    retention_days: int
    total: StatisticsSummaryView
    groups: list[StatisticsGroupView]


class PricingInput(Input):
    input: float | None = Field(default=None, ge=0)
    output: float | None = Field(default=None, ge=0)
    cache_read: float | None = Field(default=None, ge=0)


class UsageSummaryView(BaseModel):
    requests: int
    input_tokens: int
    output_tokens: int
    cached_tokens: int
    total_tokens: int
    cost_usd: float | None = None
    priced_requests: int
    unpriced_requests: int
    estimated_requests: int


class UsageDayView(BaseModel):
    date: str
    requests: int
    input_tokens: int
    output_tokens: int
    total_tokens: int
    cost_usd: float | None = None


class UsageGroupView(BaseModel):
    key: str
    model_id: str
    provider_id: str
    model_name: str
    mode: str
    requests: int
    input_tokens: int
    output_tokens: int
    cached_tokens: int
    total_tokens: int
    cost_usd: float | None = None
    pricing_source: str
    unpriced: bool
    provider_name: str | None = None


class UsageProviderView(BaseModel):
    provider_id: str
    provider_name: str | None = None
    requests: int
    input_tokens: int
    output_tokens: int
    cached_tokens: int
    total_tokens: int
    cost_usd: float | None = None
    unpriced: bool


class UsageView(BaseModel):
    retention_days: int
    total: UsageSummaryView
    days: list[UsageDayView]
    groups: list[UsageGroupView]
    providers: list[UsageProviderView]


class DiagnosticsFailureView(BaseModel):
    status: Literal["failed", "interrupted"]
    mode: str
    created_at: str


class DiagnosticsRunsView(BaseModel):
    active: int
    failed_30d: int
    interrupted_30d: int
    stale_active: int
    recent_failures: list[DiagnosticsFailureView]


class DiagnosticsView(BaseModel):
    checked_at: str
    services: dict[str, str]
    runs: DiagnosticsRunsView


class ToolView(BaseModel):
    id: str
    default_permission: Literal["allow", "ask", "deny"] = "ask"

    model_config = ConfigDict(extra="allow")


class PermissionRuleDataView(BaseModel):
    agent_id: str = ""
    tool_id: str
    permission: Literal["allow", "ask", "deny"]
    scope: dict = Field(default_factory=dict)

    model_config = ConfigDict(extra="allow")


class PermissionRuleView(BaseModel):
    id: str
    data: PermissionRuleDataView
    created_at: str


class ModelSyncResult(BaseModel):
    status: Literal["ok", "failed", "skipped"]
    count: int
    auto_count: int
    error: str | None = None


class ProviderView(RecordView):
    model_sync: ModelSyncResult


class Credentials(Input):
    username: str = Field(min_length=1, max_length=100)
    password: str = Field(min_length=12, max_length=256)


class PasswordChangeInput(Input):
    current_password: str = Field(min_length=12, max_length=256)
    new_password: str = Field(min_length=12, max_length=256)
    revoke_all_sessions: bool = False


class UsernameChangeInput(Input):
    current_password: str = Field(min_length=12, max_length=256)
    username: str = Field(min_length=1, max_length=100)


class SessionRevocationInput(Input):
    scope: Literal["others", "all"]


class ProviderInput(Input):
    name: str = Field(min_length=1, max_length=100)
    # kind remains accepted for records created before the catalog existed.
    kind: Literal["openai", "anthropic", "gemini", "openrouter", "ollama", "lmstudio", "compatible"] | None = None
    preset_id: str | None = Field(default=None, max_length=100)
    base_url: str = Field(default="", max_length=2000)
    api_key: str | None = Field(default=None, max_length=2000)
    extra_config: dict[str, str | int | float | bool] = Field(default_factory=dict)
    allow_private: bool = False
    rate_limit_rpm: int = Field(default=0, ge=0, le=10000)
    rate_limit_period: Literal["minute", "second"] = "minute"


class ModelInput(Input):
    provider_id: str
    model_id: str = Field(min_length=1, max_length=200)
    name: str = Field(default="", max_length=200)
    capabilities: dict[str, bool | None] = Field(default_factory=dict)
    overrides: dict[str, bool | None] = Field(default_factory=dict)
    context_window: int | None = Field(default=None, ge=1024, le=10000000)
    # A user-supplied limit takes precedence over catalog/provider metadata and
    # must survive the next provider refresh.
    context_window_override: int | None = Field(default=None, ge=1024, le=10000000)
    # USD per 1,000,000 tokens.  An administrator override beats catalog data.
    pricing_override: PricingInput | None = None
    source: str = "manual"


class AgentInput(Input):
    name: str = Field(min_length=1, max_length=100)
    system_prompt: str = Field(default="You are MIX, a helpful assistant.", max_length=50000)
    model_id: str = ""
    mode: Literal["chat", "thinking", "agent"] = "agent"
    tool_ids: list[str] = Field(default_factory=list, max_length=100)
    memory_scopes: list[str] = Field(default_factory=lambda: ["user"], max_length=20)
    skill_ids: list[str] = Field(default_factory=list, max_length=100)
    auto_learn: bool = True
    max_steps: int = Field(default=200, ge=1, le=2000)
    max_seconds: int = Field(default=3600, ge=30, le=86400)
    max_tool_calls: int = Field(default=500, ge=1, le=5000)
    model_settings: dict = Field(default_factory=dict)
    tool_settings: dict[str, dict[str, object]] = Field(default_factory=dict)


class ConversationInput(Input):
    title: str = Field(default="新しいチャット", max_length=200)
    folder_id: str | None = None
    project_id: str | None = None
    pinned: bool = False


class ProjectInput(Input):
    name: str = Field(min_length=1, max_length=100)
    instructions: str = Field(default="", max_length=20000)
    sources: list[str] = Field(default_factory=list, max_length=20)


class ScheduledJobInput(Input):
    name: str = Field(min_length=1, max_length=100)
    target_type: Literal["agent", "conversation"]
    target_id: str = Field(min_length=1, max_length=100)
    prompt: str = Field(min_length=1, max_length=100000)
    cron: str = Field(min_length=9, max_length=100)
    timezone: str = Field(default="Asia/Tokyo", max_length=100)
    enabled: bool = True
    catch_up: bool = False


class NotificationReadInput(Input):
    read: bool = True


class ConversationFolderInput(Input):
    name: str = Field(min_length=1, max_length=100)


class ConversationStateInput(Input):
    folder_id: str | None = None
    project_id: str | None = None
    pinned: bool | None = None
    archived: bool | None = None


class ConversationDeleteInput(Input):
    permanent: bool = False


class MessageInput(Input):
    content: str = Field(max_length=100000)
    # Conversation-first clients may omit this and reuse the saved selection.
    # Existing API clients can continue to send it explicitly.
    model_id: str = ""
    agent_id: str = ""
    mode: Literal["chat", "thinking", "agent"] = "chat"
    artifact_ids: list[str] = Field(default_factory=list, max_length=5)
    temporary_mode: bool = False
    allow_tools: bool = False
    research_mode: bool = False


class DecisionInput(Input):
    decision: Literal["once", "always", "denied"]


class MemoryInput(Input):
    content: str = Field(min_length=1, max_length=10000)
    scope: str = Field(default="user", max_length=100)
    gist: str | None = Field(default=None, max_length=1000)
    strength: float | None = Field(default=None, ge=0, le=1)
    confidence: float | None = Field(default=None, ge=0, le=1)
    salience: float | None = Field(default=None, ge=0, le=1)
    lifecycle_state: Literal[
        "candidate", "active", "superseded", "expired", "archived", "disputed",
        # legacy aliases kept for backward compatibility
        "latent", "established", "deleted",
    ] | None = None
    entities: list[str] | None = Field(default=None, max_length=50)
    concepts: list[str] | None = Field(default=None, max_length=50)
    temporal_context: str | None = Field(default=None, max_length=200)
    metadata: dict | None = None
    # Role-based taxonomy
    role: Literal[
        "fact", "decision", "preference", "goal", "constraint",
        "experience", "failure", "solution", "observation",
        "hypothesis", "question",
    ] | None = None
    source_kind: Literal[
        "user", "agent", "tool", "browser", "external", "consolidation",
    ] | None = None
    role_metadata: dict | None = None
    verification: Literal[
        "unverified", "pending", "verified", "disputed", "rejected",
    ] | None = None
    # Deprecated compatibility inputs. Category is retained only as legacy metadata.
    importance: int = Field(default=2, ge=1, le=5)
    category: str | None = Field(default=None, max_length=50)
    pinned: bool = False


class MemoryEvidenceInput(Input):
    kind: Literal[
        "user_message", "conversation", "tool_result", "browser",
        "file", "code_change", "external", "agent_action", "observation",
    ]
    ref: str = Field(default="", max_length=500)
    summary: str = Field(default="", max_length=2000)
    confidence: float | None = Field(default=None, ge=0, le=1)
    data: dict | None = None


class MemoryResolveConflictInput(Input):
    resolution: Literal["prefer_a", "prefer_b", "merge", "dismiss"]
    prefer: str | None = Field(default=None, max_length=36)


class MemoryRememberInput(Input):
    """Reduced-shape ``memory.remember`` tool surface."""
    content: str = Field(min_length=1, max_length=10000)
    role: Literal[
        "fact", "decision", "preference", "goal", "constraint",
        "experience", "failure", "solution", "observation",
        "hypothesis", "question",
    ] = "fact"
    scope: Literal["working", "task", "project", "user", "world"] = "task"
    source_kind: Literal[
        "user", "agent", "tool", "browser", "external", "consolidation",
    ] | None = None
    role_metadata: dict | None = None
    entities: list[str] | None = Field(default=None, max_length=50)
    concepts: list[str] | None = Field(default=None, max_length=50)
    evidence: list[dict] | None = Field(default=None, max_length=20)
    task_id: str | None = Field(default=None, max_length=36)
    explicit_user: bool = False


class SkillInput(Input):
    name: str = Field(min_length=1, max_length=100)
    description: str = Field(default="", max_length=1000)
    content: str = Field(min_length=1, max_length=50000)
    enabled: bool = True


class SkillImportInput(Input):
    text: str = Field(min_length=1, max_length=200000)


class MCPInput(Input):
    name: str = Field(min_length=1, max_length=100)
    transport: Literal["stdio", "http"]
    url: str = Field(default="", max_length=2000)
    command: str = Field(default="", max_length=1000)
    args: list[str] = Field(default_factory=list, max_length=50)
    credentials: dict[str, dict[str, str]] | None = None
    enabled: bool = True


class MCPInstallInput(Input):
    registry_id: str = Field(min_length=1, max_length=255)
    version: str = Field(default="latest", min_length=1, max_length=255)
    network_capability: Literal["none", "restricted", "public_web"] = "none"
    allowed_domains: list[str] = Field(default_factory=list, max_length=100)
    configuration: dict[str, str] = Field(default_factory=dict)
    secrets: dict[str, str] | None = None


class MCPOAuthStartInput(Input):
    client_id: str = Field(default="", max_length=2000)
    client_secret: str = Field(default="", max_length=4000)
    scopes: list[str] = Field(default_factory=list, max_length=100)


class MCPUninstallInput(Input):
    delete_volume: bool
    confirm_volume: str = Field(default="", max_length=255)


class SettingsInput(Input):
    ui_language: Literal["ja", "en"] = "ja"
    default_model_id: str = ""
    auto_model_ids: list[str] | None = None
    auto_retry_count: int = Field(default=3, ge=0)
    auto_dynamic_switching: bool = True
    auto_priority: Literal["balanced", "quality", "speed", "cost"] = "balanced"
    setup_complete: bool = False
    brave_api_key: str | None = None
    tavily_api_key: str | None = None
    exa_api_key: str | None = None
    serper_api_key: str | None = None
    searxng_url: str | None = None
    web_search_backend: Literal["ddgs", "brave", "tavily", "exa", "serper", "searxng"] = "ddgs"
    allowed_domains: list[str] = Field(default_factory=list, max_length=100)
    browser_enabled: bool = True
    browser_install_requested: bool = False
    browser_install_status: Literal["not_installed", "installing", "ready", "failed"] = "not_installed"
    browser_install_failure: str | None = None
    browser_install_progress: int | None = None
    browser_install_stage: str | None = None
    browser_timeout_ms: int = Field(default=15000, ge=3000, le=60000)
    browser_locale: str = Field(default="ja-JP", min_length=2, max_length=35)
    browser_user_agent: str = Field(default="", max_length=500)
    browser_viewport_width: int = Field(default=1280, ge=320, le=2560)
    browser_viewport_height: int = Field(default=720, ge=320, le=2560)
    browser_block_images: bool = False
    web_search_enabled: bool = True
    web_search_count: int = Field(default=5, ge=1, le=20)
    tool_settings: dict[str, dict[str, object]] = Field(default_factory=dict)
    memory_auto_formation: bool = True
    memory_seed_limit: int = Field(default=24, ge=4, le=64)
    memory_result_limit: int = Field(default=8, ge=1, le=30)


class FeedbackInput(Input):
    value: Literal["up", "down"] | None = None


class PermissionInput(Input):
    agent_id: str = ""
    tool_id: str
    permission: Literal["allow", "ask", "deny"]
    scope: dict = Field(default_factory=dict)


class ResumeInput(Input):
    acknowledge_unknown_result: bool = False
    max_seconds: int | None = Field(default=None, ge=30, le=86400)
    max_steps: int | None = Field(default=None, ge=1, le=2000)
    max_tool_calls: int | None = Field(default=None, ge=1, le=5000)
    from_checkpoint: str | None = Field(default=None, max_length=64)
    unknown_actions: list[dict] | None = Field(default=None, max_length=50)


class BudgetExtensionDecisionInput(Input):
    grant: bool
    max_seconds: int | None = Field(default=None, ge=30, le=86400)
    max_steps: int | None = Field(default=None, ge=1, le=2000)
    max_tool_calls: int | None = Field(default=None, ge=1, le=5000)


class BackupInput(Input):
    passphrase: str = Field(min_length=12, max_length=256)
