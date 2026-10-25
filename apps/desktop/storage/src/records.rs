//! Typed record wrappers over the JSONB-style `data` columns.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const SINGLE_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record<T> {
    pub id: String,
    pub owner_id: String,
    pub data: T,
    pub created_at: DateTime<Utc>,
}

impl<T> Record<T> {
    pub fn new(data: T) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: SINGLE_USER_ID.to_string(),
            data,
            created_at: Utc::now(),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderData {
    #[serde(rename = "kind")]
    pub kind: String,
    #[serde(rename = "label")]
    pub label: String,
    #[serde(rename = "baseUrl", default)]
    pub base_url: Option<String>,
    #[serde(rename = "enabled")]
    pub enabled: bool,
    #[serde(rename = "preset", default)]
    pub preset: Option<String>,
    #[serde(rename = "extra", default)]
    pub extra: serde_json::Value,
}

pub type ProviderRecord = Record<ProviderData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelData {
    #[serde(rename = "providerId")]
    pub provider_id: String,
    #[serde(rename = "label")]
    pub label: String,
    #[serde(rename = "contextWindow")]
    pub context_window: u32,
    #[serde(rename = "vision")]
    pub vision: bool,
    #[serde(rename = "toolCalling")]
    pub tool_calling: bool,
    #[serde(rename = "reasoning")]
    pub reasoning: bool,
    #[serde(rename = "auto", default)]
    pub auto: bool,
}

pub type ModelRecord = Record<ModelData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettingsData {
    #[serde(rename = "defaultModelId", default)]
    pub default_model_id: Option<String>,
    #[serde(rename = "defaultMode", default)]
    pub default_mode: Option<String>,
    #[serde(rename = "egressAllowlist", default)]
    pub egress_allowlist: Vec<String>,
    #[serde(rename = "autoUpdate", default)]
    pub auto_update: bool,
    #[serde(rename = "telemetryConsent", default)]
    pub telemetry_consent: bool,
    /// Default parent folder used when auto-generating project workspaces
    /// (Projectless chats, legacy backfills). `None` →
    /// `~/Documents/MIX-agent Projects`.
    #[serde(rename = "projectsRoot", default)]
    pub projects_root: Option<String>,
    /// Codex-compatible `project_doc_max_bytes`: combined byte cap for the
    /// `AGENTS.md` chain. `None` → 32 KiB.
    #[serde(rename = "projectDocMaxBytes", default)]
    pub project_doc_max_bytes: Option<u64>,
    /// Codex-compatible `project_doc_fallback_filenames`: extra instruction
    /// filenames consulted after `AGENTS.override.md` / `AGENTS.md`.
    #[serde(rename = "projectDocFallbackFilenames", default)]
    pub project_doc_fallback_filenames: Vec<String>,
}

pub type SettingsRecord = Record<SettingsData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentData {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "instructions")]
    pub instructions: String,
    #[serde(rename = "toolIds", default)]
    pub tool_ids: Vec<String>,
}

pub type AgentRecord = Record<AgentData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceData {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "instructions", default)]
    pub instructions: String,
    #[serde(rename = "documents", default)]
    pub documents: Vec<WorkspaceDocument>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceDocument {
    #[serde(rename = "filename")]
    pub filename: String,
    #[serde(rename = "content")]
    pub content: String,
}

pub type WorkspaceRecord = Record<WorkspaceData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationData {
    #[serde(rename = "title")]
    pub title: String,
    #[serde(rename = "mode")]
    pub mode: String,
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "folderId", default)]
    pub folder_id: Option<String>,
    #[serde(rename = "projectId", default)]
    pub project_id: Option<String>,
    #[serde(rename = "workspaceId", default)]
    pub workspace_id: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
    #[serde(rename = "state", default)]
    pub state: String,
}

pub type ConversationRecord = Record<ConversationData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationFolderData {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "sortIndex", default)]
    pub sort_index: i32,
}

pub type ConversationFolderRecord = Record<ConversationFolderData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageData {
    #[serde(rename = "role")]
    pub role: String,
    #[serde(rename = "content")]
    pub content: String,
    #[serde(rename = "modelId", default)]
    pub model_id: Option<String>,
    #[serde(rename = "runId", default)]
    pub run_id: Option<String>,
    #[serde(rename = "toolCalls", default)]
    pub tool_calls: Vec<String>,
    #[serde(rename = "attachments", default)]
    pub attachments: Vec<MessageAttachment>,
    #[serde(rename = "kind", default)]
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageAttachment {
    #[serde(rename = "kind")]
    pub kind: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "dataBase64")]
    pub data_base64: String,
    #[serde(rename = "filename", default)]
    pub filename: Option<String>,
}

pub type MessageRecord = Record<MessageData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectData {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "instructions")]
    pub instructions: String,
    /// Absolute path of the folder this project is rooted at. Required since
    /// the folder-first refactor; the data migration
    /// [`crate::migrations`] assigns a generated folder to legacy rows that
    /// had `null`.
    #[serde(rename = "rootPath")]
    pub root_path: String,
    /// Extra directories the agent may read from but not write to. Parity with
    /// Codex's `--add-dir` flag for monorepos.
    #[serde(rename = "additionalPaths", default)]
    pub additional_paths: Vec<String>,
    /// Pasted reference texts (parity with the Docker edition).
    #[serde(rename = "sources", default)]
    pub sources: Vec<String>,
    /// The project's own check (`pnpm rust:test`). When set, a Run that changed
    /// files may not conclude until this command has succeeded. `None` falls
    /// back to the app-wide setting, and then to no gate at all.
    #[serde(rename = "verifyCommand", default)]
    pub verify_command: Option<String>,
    /// Formatter or lint command reported to the model around edits.
    #[serde(rename = "formatCommand", default)]
    pub format_command: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
}

pub type ProjectRecord = Record<ProjectData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunData {
    #[serde(rename = "mode")]
    pub mode: String,
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "status")]
    pub status: String,
    #[serde(rename = "stepCount", default)]
    pub step_count: u32,
    #[serde(rename = "toolCallCount", default)]
    pub tool_call_count: u32,
    #[serde(rename = "startedAt")]
    pub started_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
    #[serde(rename = "summary", default)]
    pub summary: String,
    #[serde(rename = "budget", default)]
    pub budget: serde_json::Value,
}

pub type RunRecord = Record<RunData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunCheckpointData {
    #[serde(rename = "history", default)]
    pub history: serde_json::Value,
    #[serde(rename = "taskState", default)]
    pub task_state: serde_json::Value,
    #[serde(rename = "agentPlan", default)]
    pub agent_plan: serde_json::Value,
    #[serde(rename = "summary", default)]
    pub summary: serde_json::Value,
    #[serde(rename = "recentToolCalls", default)]
    pub recent_tool_calls: serde_json::Value,
    #[serde(rename = "counters", default)]
    pub counters: serde_json::Value,
    #[serde(rename = "trigger")]
    pub trigger: String,
}

pub type RunCheckpointRecord = Record<RunCheckpointData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolData {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "description")]
    pub description: String,
    #[serde(rename = "builtin")]
    pub builtin: bool,
    #[serde(rename = "defaultPermission")]
    pub default_permission: String,
    #[serde(rename = "parameters")]
    pub parameters: serde_json::Value,
    #[serde(rename = "category", default)]
    pub category: String,
}

pub type ToolRecord = Record<ToolData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCallData {
    #[serde(rename = "toolName")]
    pub tool_name: String,
    #[serde(rename = "arguments")]
    pub arguments: serde_json::Value,
    #[serde(rename = "result", default)]
    pub result: serde_json::Value,
    #[serde(rename = "error", default)]
    pub error: Option<String>,
    #[serde(rename = "startedAt")]
    pub started_at: DateTime<Utc>,
    #[serde(rename = "finishedAt", default)]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(rename = "approvalId", default)]
    pub approval_id: Option<String>,
}

pub type ToolCallRecord = Record<ToolCallData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApprovalData {
    #[serde(rename = "toolName")]
    pub tool_name: String,
    #[serde(rename = "arguments")]
    pub arguments: serde_json::Value,
    #[serde(rename = "grantedAt", default)]
    pub granted_at: Option<DateTime<Utc>>,
    #[serde(rename = "remember", default)]
    pub remember: bool,
}

pub type ApprovalRecord = Record<ApprovalData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionRuleData {
    #[serde(rename = "toolId")]
    pub tool_id: String,
    #[serde(rename = "scope")]
    pub scope: String,
    #[serde(rename = "decision")]
    pub decision: String,
    #[serde(rename = "pattern", default)]
    pub pattern: Option<String>,
}

pub type PermissionRuleRecord = Record<PermissionRuleData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryItemData {
    #[serde(rename = "title")]
    pub title: String,
    #[serde(rename = "content")]
    pub content: String,
    #[serde(rename = "role")]
    pub role: String,
    #[serde(rename = "scope")]
    pub scope: String,
    #[serde(rename = "sourceKind")]
    pub source_kind: String,
    #[serde(rename = "verification")]
    pub verification: String,
}

pub type MemoryItemRecord = Record<MemoryItemData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryRevisionData {
    #[serde(rename = "memoryId")]
    pub memory_id: String,
    #[serde(rename = "title")]
    pub title: String,
    #[serde(rename = "content")]
    pub content: String,
}

pub type MemoryRevisionRecord = Record<MemoryRevisionData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillData {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "description")]
    pub description: String,
    #[serde(rename = "bodyMarkdown")]
    pub body_markdown: String,
    #[serde(rename = "resources", default)]
    pub resources: serde_json::Value,
    #[serde(rename = "allowedTools", default)]
    pub allowed_tools: serde_json::Value,
    #[serde(rename = "source", default)]
    pub source: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
}

pub type SkillRecord = Record<SkillData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillRevisionData {
    #[serde(rename = "skillId")]
    pub skill_id: String,
    #[serde(rename = "body")]
    pub body: serde_json::Value,
}

pub type SkillRevisionRecord = Record<SkillRevisionData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeData {
    #[serde(rename = "docId")]
    pub doc_id: String,
    #[serde(rename = "filename")]
    pub filename: String,
    #[serde(rename = "chunkIndex")]
    pub chunk_index: u32,
    #[serde(rename = "text")]
    pub text: String,
}

pub type KnowledgeRecord = Record<KnowledgeData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtifactData {
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "kind")]
    pub kind: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "label", default)]
    pub label: Option<String>,
    #[serde(rename = "path")]
    pub path: String,
    #[serde(rename = "size", default)]
    pub size: u64,
}

pub type ArtifactRecord = Record<ArtifactData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduledJobData {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "cron")]
    pub cron: String,
    #[serde(rename = "prompt")]
    pub prompt: String,
    #[serde(rename = "conversationId", default)]
    pub conversation_id: Option<String>,
    #[serde(rename = "enabled")]
    pub enabled: bool,
    #[serde(rename = "lastRunAt", default)]
    pub last_run_at: Option<DateTime<Utc>>,
}

pub type ScheduledJobRecord = Record<ScheduledJobData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduledRunData {
    #[serde(rename = "jobId")]
    pub job_id: String,
    #[serde(rename = "scheduledAt")]
    pub scheduled_at: DateTime<Utc>,
    #[serde(rename = "status")]
    pub status: String,
    #[serde(rename = "runId", default)]
    pub run_id: Option<String>,
}

pub type ScheduledRunRecord = Record<ScheduledRunData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationData {
    #[serde(rename = "kind")]
    pub kind: String,
    #[serde(rename = "title")]
    pub title: String,
    #[serde(rename = "body")]
    pub body: String,
    #[serde(rename = "level", default)]
    pub level: String,
    #[serde(rename = "readAt", default)]
    pub read_at: Option<DateTime<Utc>>,
}

pub type NotificationRecord = Record<NotificationData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutoReliabilityEventData {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "outcome")]
    pub outcome: String,
    #[serde(rename = "reason", default)]
    pub reason: Option<String>,
}

pub type AutoReliabilityEventRecord = Record<AutoReliabilityEventData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerformanceEventData {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "latencyMs")]
    pub latency_ms: u64,
    #[serde(rename = "inputTokens", default)]
    pub input_tokens: u64,
    #[serde(rename = "outputTokens", default)]
    pub output_tokens: u64,
    #[serde(rename = "toolCalls", default)]
    pub tool_calls: u32,
    #[serde(rename = "outcome")]
    pub outcome: String,
}

pub type PerformanceEventRecord = Record<PerformanceEventData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsageEventData {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(rename = "providerId")]
    pub provider_id: String,
    #[serde(rename = "mode")]
    pub mode: String,
    #[serde(rename = "inputTokens", default)]
    pub input_tokens: u64,
    #[serde(rename = "outputTokens", default)]
    pub output_tokens: u64,
    #[serde(rename = "cacheReadTokens", default)]
    pub cache_read_tokens: u64,
    #[serde(rename = "cacheWriteTokens", default)]
    pub cache_write_tokens: u64,
}

pub type ModelUsageEventRecord = Record<ModelUsageEventData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConnectionData {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "transport")]
    pub transport: String,
    #[serde(rename = "command", default)]
    pub command: Option<String>,
    #[serde(rename = "args", default)]
    pub args: Vec<String>,
    #[serde(rename = "url", default)]
    pub url: Option<String>,
    #[serde(rename = "headers", default)]
    pub headers: std::collections::HashMap<String, String>,
    #[serde(rename = "enabled")]
    pub enabled: bool,
    #[serde(rename = "status", default)]
    pub status: String,
    #[serde(rename = "clientId", default)]
    pub client_id: Option<String>,
    #[serde(rename = "useDocker", default)]
    pub use_docker: bool,
    #[serde(rename = "image", default)]
    pub image: Option<String>,
}

pub type McpConnectionRecord = Record<McpConnectionData>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpAuthStateData {
    #[serde(rename = "connectionId")]
    pub connection_id: String,
    #[serde(rename = "verifier")]
    pub verifier: String,
    #[serde(rename = "state")]
    pub state: String,
    #[serde(rename = "redirectUri")]
    pub redirect_uri: String,
}

pub type McpAuthStateRecord = Record<McpAuthStateData>;
