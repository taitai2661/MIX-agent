import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentView,
  AttachmentPayload,
  BrowserInvokeOutput,
  BrowserProfileView,
  BrowserStatus,
  CheckpointView,
  ConversationSummary,
  DiagnosticsExport,
  DiagnosticsReport,
  DirEntryView,
  DiscoveredModelView,
  FileContentView,
  FileDiffView,
  GitDiffView,
  GitStatusView,
  HealthReport,
  KnowledgeDocView,
  KnowledgeSearchHit,
  LocalRuntimeView,
  LocalScanView,
  McpConnectionView,
  ConflictResolutionView,
  MemoryLinkView,
  MemoryRevisionView,
  MemorySearchHit,
  MemoryView,
  MessageView,
  ModelView,
  PendingApprovalView,
  PermissionRuleView,
  ProjectView,
  ProviderPresetView,
  ProviderView,
  RunEventView,
  RunView,
  ScheduleJobView,
  SendMessageOutput,
  SettingsView,
  SkillResourceView,
  SkillView,
  ToolView,
} from "./types";

export const api = {
  health: () => invoke<HealthReport>("health"),

  listProviders: () => invoke<ProviderView[]>("list_providers"),
  providerPresets: () => invoke<ProviderPresetView[]>("provider_presets"),
  createProvider: (input: {
    preset_slug: string | null;
    kind: string;
    label: string;
    base_url: string | null;
    api_key: string | null;
    enabled: boolean | null;
  }) => invoke<ProviderView>("create_provider", { input }),
  updateProvider: (input: {
    id: string;
    label: string | null;
    base_url: string | null;
    api_key: string | null;
    enabled: boolean | null;
  }) => invoke<ProviderView>("update_provider", { input }),
  deleteProvider: (id: string) => invoke<void>("delete_provider", { id }),
  connectProvider: (id: string) =>
    invoke<number>("connect_provider", { input: { id, discover: true } }),

  scanLocalRuntimes: () => invoke<LocalScanView>("scan_local_runtimes"),
  probeLocalEndpoint: (baseUrl: string) =>
    invoke<LocalRuntimeView>("probe_local_endpoint", { baseUrl }),
  importLocalRuntime: (input: {
    slug: string;
    kind: string;
    label: string;
    base_url: string;
    models: DiscoveredModelView[];
  }) =>
    invoke<{ provider: ProviderView; models: ModelView[] }>(
      "import_local_runtime",
      { input },
    ),

  listModels: () => invoke<ModelView[]>("list_models", { providerId: null }),
  verifyTools: (modelId: string) =>
    invoke<ModelView>("verify_tools", { modelId }),
  setModelAuto: (modelId: string, auto: boolean) =>
    invoke<ModelView>("set_model_auto", { modelId, auto }),
  setMessageFeedback: (messageId: string, value: string | null) =>
    invoke<string | null>("set_message_feedback", { messageId, value }),

  listConversations: () => invoke<ConversationSummary[]>("list_conversations"),
  createConversation: (input: {
    mode: string;
    model_id: string;
    title: string | null;
    workspace_id: string | null;
    project_id?: string | null;
  }) => invoke<ConversationSummary>("create_conversation", { input }),
  setConversationProject: (conversationId: string, projectId: string | null) =>
    invoke<void>("set_conversation_project", { conversationId, projectId }),
  sendMessage: (input: {
    conversation_id: string;
    content: string;
    model_id: string | null;
    mode: string | null;
    workspace_id: string | null;
    attachments: Array<{
      kind: string;
      mime_type: string;
      data_base64: string;
      filename: string | null;
    }>;
    reasoning: string | null;
    temperature: number | null;
    max_tokens: number | null;
    /** Temporary mode: no Memory retrieval, transcript purged after the run. */
    temporary_mode?: boolean;
    /** Only with `temporary_mode`: expose tools for this turn. */
    allow_tools?: boolean;
  }) => invoke<SendMessageOutput>("send_message", { input }),
  editMessage: (input: {
    conversation_id: string;
    message_id: string;
    content: string;
    model_id: string | null;
    mode: string | null;
    reasoning: string | null;
  }) => invoke<SendMessageOutput>("edit_message", { input }),
  listMessages: (conversationId: string, limit?: number) =>
    invoke<MessageView[]>("list_messages", {
      conversationId,
      limit: limit ?? null,
    }),
  renameConversation: (id: string, title: string) =>
    invoke<void>("rename_conversation", { id, title }),
  deleteConversation: (id: string) =>
    invoke<void>("delete_conversation", { id }),
  exportMarkdown: (id: string, includeToolCalls: boolean) =>
    invoke<string>("export_markdown", { id, includeToolCalls }),

  cancelRun: (id: string) => invoke<void>("cancel_run", { id }),
  getRun: (id: string) => invoke<RunView>("get_run", { id }),
  listRuns: (conversationId: string) =>
    invoke<RunView[]>("list_runs", { conversationId }),
  listRunEvents: (runId: string) =>
    invoke<RunEventView[]>("list_run_events", { runId }),
  listCheckpoints: (id: string) =>
    invoke<CheckpointView[]>("list_checkpoints", { id }),
  replayRunEvents: (id: string) => invoke<void>("replay_run_events", { id }),
  resumeRun: (input: {
    id: string;
    from_checkpoint: string | null;
    unknown_actions: Array<{ tool_call_id: string; action: string }>;
  }) => invoke<string>("resume_run", { input }),
  budgetExtension: (input: {
    id: string;
    grant: boolean;
    step_limit: number | null;
    tool_call_limit: number | null;
    wall_clock_minutes: number | null;
  }) => invoke<void>("budget_extension", { input }),

  listTools: () => invoke<ToolView[]>("list_tools"),
  setPermissionRule: (input: {
    tool_id: string;
    scope: string;
    decision: string;
    pattern: string | null;
    workspace_id?: string | null;
  }) => invoke<void>("set_permission_rule", { input }),
  listPermissionRules: () =>
    invoke<PermissionRuleView[]>("list_permission_rules"),
  listPendingApprovals: () =>
    invoke<PendingApprovalView[]>("list_pending_approvals"),
  decideApproval: (input: { id: string; grant: boolean; remember: boolean }) =>
    invoke<void>("decide_approval", { input }),

  listMcpConnections: () => invoke<McpConnectionView[]>("list_connections"),
  createMcpConnection: (input: {
    name: string;
    transport: string;
    command: string | null;
    args: string[] | null;
    url: string | null;
    headers: Record<string, string> | null;
  }) => invoke<McpConnectionView>("create_connection", { input }),
  deleteMcpConnection: (id: string) =>
    invoke<void>("delete_connection", { id }),
  connectMcp: () => invoke<number>("connect"),
  startMcpOauth: (connectionId: string) =>
    invoke<{ authorization_url: string; state: string }>("start_oauth", {
      connectionId,
    }),
  handleMcpOauthCallback: (url: string) =>
    invoke<void>("handle_oauth_callback", { url }),

  browserStatus: () => invoke<BrowserStatus>("browser_status"),
  browserInvoke: (input: {
    workspace_id: string | null;
    action: string;
    url: string | null;
    selector: string | null;
    text: string | null;
    x: number | null;
    y: number | null;
    script: string | null;
    full_page: boolean | null;
    condition: string | null;
    timeout_ms: number | null;
  }) => invoke<BrowserInvokeOutput>("browser_invoke", { input }),
  browserProfiles: () => invoke<BrowserProfileView[]>("browser_profiles"),
  browserResetProfile: (workspaceId: string) =>
    invoke<void>("browser_reset_profile", { workspaceId }),

  listMemories: () => invoke<MemoryView[]>("list_memories"),
  createMemory: (input: {
    title: string;
    body: string;
    role: string | null;
    scope: string | null;
  }) => invoke<MemoryView>("create_memory", { input }),
  updateMemory: (input: { id: string; title: string; body: string }) =>
    invoke<MemoryView>("update_memory", { input }),
  deleteMemory: (id: string) => invoke<void>("delete_memory", { id }),
  listMemoryRevisions: (memoryId: string) =>
    invoke<MemoryRevisionView[]>("list_memory_revisions", { memoryId }),
  restoreMemoryRevision: (memoryId: string, revisionId: string) =>
    invoke<MemoryView>("restore_memory_revision", { memoryId, revisionId }),
  // §7.2 `recall`'s shape — `content`, `score`, §4's `selection_reason` — and
  // not `MemoryView`, which is §1's row under a different set of names.
  searchMemories: (query: string, limit: number | null) =>
    invoke<MemorySearchHit[]>("search_memories", { query, limit }),
  // `verdict` is required by the Rust command; omitting it made Tauri fail to
  // deserialize the arguments, so the verify action always rejected.
  verifyMemory: (id: string, verdict: string) =>
    invoke<MemoryView>("verify_memory", { id, verdict }),
  /** §4's reference API: every edge touching one Memory, both directions. */
  memoryAssociations: (memoryId: string, limit?: number) =>
    invoke<MemoryLinkView[]>("memory_associations", {
      memoryId,
      limit: limit ?? null,
    }),
  /**
   * §3 `resolve_conflict`. `prefer` is `null` to keep §3's own default — **the
   * already-stored side wins** — and one of the two ids to adopt that side
   * explicitly. `resolution` decides the loser's fate: `dismiss` / `rejected` /
   * `discard` make it `expired`, anything else `superseded`.
   */
  resolveConflict: (input: {
    conflict_id: string;
    resolution: string;
    prefer: string | null;
  }) => invoke<ConflictResolutionView>("resolve_conflict", { ...input }),

  listKnowledge: () => invoke<KnowledgeDocView[]>("list_knowledge"),
  ingestKnowledge: (input: { filename: string; text: string }) =>
    invoke<KnowledgeDocView>("ingest_knowledge", { input }),
  importKnowledgeFile: (input: { path: string }) =>
    invoke<KnowledgeDocView>("import_knowledge_file", { input }),
  searchKnowledge: (query: string, limit: number | null) =>
    invoke<KnowledgeSearchHit[]>("search_knowledge", { query, limit }),
  deleteKnowledge: (docId: string) =>
    invoke<number>("delete_knowledge", { docId }),

  listSkills: () => invoke<SkillView[]>("list_skills"),
  importSkill: (input: {
    source: string;
    payload: string | null;
    path: string | null;
    name: string | null;
  }) => invoke<SkillView>("import_skill", { input }),
  exportSkill: (name: string, asZip: boolean) =>
    invoke<string>("export_skill", { name, asZip }),
  deleteSkill: (name: string) => invoke<void>("delete_skill", { name }),
  skillResources: (name: string) =>
    invoke<SkillResourceView[]>("list_skill_resources", { name }),
  readSkillResource: (name: string, path: string) =>
    invoke<SkillResourceView>("read_skill_resource", { name, path }),

  listProjects: () => invoke<ProjectView[]>("list_projects"),
  // Folder-first: `rootPath` must be an existing directory, `name` may be
  // omitted and then defaults to the folder's basename.
  createProject: (input: {
    name?: string;
    instructions?: string;
    rootPath: string;
    additionalPaths?: string[];
    sources?: string[];
  }) => invoke<ProjectView>("create_project", { input }),
  updateProject: (
    id: string,
    input: {
      name?: string;
      instructions?: string;
      rootPath?: string;
      additionalPaths?: string[];
      sources?: string[];
    },
  ) => invoke<ProjectView>("update_project", { id, input }),
  deleteProject: (id: string) => invoke<void>("delete_project", { id }),

  listDir: (rootPath: string, path: string) =>
    invoke<DirEntryView[]>("list_dir", { rootPath, path }),
  findWorkspaceFiles: (
    rootPath: string,
    query?: string | null,
    limit?: number | null,
  ) =>
    invoke<string[]>("find_workspace_files", {
      rootPath,
      query: query ?? null,
      limit: limit ?? null,
    }),
  readWorkspaceFile: (rootPath: string, path: string) =>
    invoke<FileContentView>("read_workspace_file", { rootPath, path }),
  gitStatus: (rootPath: string) =>
    invoke<GitStatusView>("git_status", { rootPath }),
  gitDiff: (rootPath: string, scope: string, path: string | null) =>
    invoke<GitDiffView>("git_diff", { rootPath, scope, path }),
  gitRestore: (rootPath: string, path: string, staged: boolean = false) =>
    invoke<void>("git_restore", { rootPath, path, staged }),
  gitStage: (rootPath: string, path: string, unstage: boolean = false) =>
    invoke<void>("git_stage", { rootPath, path, unstage }),

  listScheduleJobs: () => invoke<ScheduleJobView[]>("list_jobs"),
  createScheduleJob: (input: {
    name: string;
    cron: string;
    prompt: string;
    conversation_id: string | null;
    model_id: string | null;
    mode: string | null;
  }) => invoke<ScheduleJobView>("create_job", { input }),
  setScheduleJobEnabled: (id: string, enabled: boolean) =>
    invoke<void>("set_job_enabled", { id, enabled }),
  previewCron: (cron: string) => invoke<string[]>("preview_cron", { cron }),
  deleteScheduleJob: (id: string) => invoke<void>("delete_job", { id }),

  createBackup: (input: { destination: string; password: string }) =>
    invoke<{ path: string; bytes: number; created_at: string; tables: number }>(
      "create_backup",
      { input },
    ),
  restoreBackup: (input: { source: string; password: string }) =>
    invoke<{
      staged_path: string;
      created_at: string;
      tables: number;
      needs_restart: boolean;
    }>("restore_backup", { input }),
  relaunchApp: () => invoke<void>("relaunch_app"),

  getSettings: () => invoke<SettingsView>("get_settings"),
  updateSettings: (input: SettingsView) =>
    invoke<SettingsView>("update_settings", { input }),

  listAgents: () => invoke<AgentView[]>("list_agents"),
  createAgent: (input: {
    name: string;
    instructions: string;
    tool_ids: string[];
  }) => invoke<AgentView>("create_agent", { input }),
  updateAgent: (input: {
    id: string;
    name: string;
    instructions: string;
    tool_ids: string[];
  }) => invoke<AgentView>("update_agent", { input }),
  // `delete_agent` was registered on the Rust side but never bound here, so
  // there was no way to remove an agent from the UI at all.
  deleteAgent: (id: string) => invoke<void>("delete_agent", { id }),

  diagnostics: () => invoke<DiagnosticsReport>("collect_diagnostics"),
  getRecentLogs: (lines?: number) =>
    invoke<string>("get_recent_logs", { lines: lines ?? null }),
  exportDiagnostics: () => invoke<DiagnosticsExport>("export_diagnostics"),

  previewFileDiff: (
    path: string,
    content: string,
    rootPath?: string | null,
    oldString?: string,
    newString?: string,
  ) =>
    invoke<FileDiffView>("preview_file_diff", {
      input: {
        path,
        content,
        root_path: rootPath ?? null,
        old_string: oldString ?? null,
        new_string: newString ?? null,
      },
    }),
  readAttachment: (path: string) =>
    invoke<AttachmentPayload>("read_attachment", { path }),
  openExternal: (url: string) => invoke<void>("open_external", { url }),
  saveTextFile: (path: string, content: string) =>
    invoke<number>("save_text_file", { path, content }),
  saveBase64File: (path: string, dataBase64: string) =>
    invoke<number>("save_base64_file", { path, dataBase64 }),
};

export interface RunEventPayload {
  run_id: string;
  sequence: number;
  kind: string;
  data: unknown;
  created_at: string;
}

export async function listenRunEvents(
  handler: (event: RunEventPayload) => void,
): Promise<UnlistenFn> {
  return listen<RunEventPayload>("run_event", (e) => handler(e.payload));
}
