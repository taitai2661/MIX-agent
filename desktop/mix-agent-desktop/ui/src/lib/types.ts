/// Shared TypeScript surface for the Tauri command payload shapes.

export interface HealthReport {
  status: string;
  storage: boolean;
  engine: boolean;
}

export interface ProviderView {
  id: string;
  kind: string;
  label: string;
  base_url: string | null;
  enabled: boolean;
  has_api_key: boolean;
  preset: string | null;
}

export interface ProviderPresetView {
  slug: string;
  label: string;
  category: string;
  kind: string;
  default_url: string;
  api_key_required: boolean;
  allow_private_default: boolean;
  notes: string;
}

export interface ModelView {
  id: string;
  provider_id: string;
  label: string;
  context_window: number;
  supports_vision: boolean;
  supports_tool_calling: boolean;
  supports_reasoning: boolean;
}

export interface DiscoveredModelView {
  id: string;
  label: string;
  context_window: number;
  parameter_size: string | null;
  quantization: string | null;
}

export interface LocalRuntimeView {
  slug: string;
  label: string;
  kind: string;
  base_url: string;
  host: string;
  port: number;
  listening: boolean;
  online: boolean;
  latency_ms: number | null;
  version: string | null;
  installed: boolean;
  install_hint: string | null;
  launcher: string | null;
  models: DiscoveredModelView[];
  error: string | null;
}

export interface OccupiedPortView {
  port: number;
  note: string | null;
}

export interface LocalScanView {
  runtimes: LocalRuntimeView[];
  occupied_ports: OccupiedPortView[];
  hosts: string[];
  elapsed_ms: number;
  any_online: boolean;
}

export interface ConversationSummary {
  id: string;
  title: string;
  mode: string;
  model_id: string;
  updated_at: string;
  preview: string | null;
  message_count: number;
  project_id: string | null;
}

export interface MessageAttachmentView {
  kind: string;
  mime_type: string;
  filename: string | null;
}

export interface MessageView {
  id: string;
  role: string;
  content: string;
  reasoning: string | null;
  model_id: string | null;
  run_id: string | null;
  kind: string;
  created_at: string;
  attachments: MessageAttachmentView[];
}

export interface RunEventView {
  run_id: string;
  sequence: number;
  kind: string;
  data: unknown;
  created_at: string;
}

export interface FileDiffView {
  path: string;
  unified_diff: string;
  added: number;
  removed: number;
  is_new: boolean;
  truncated: boolean;
}

export interface AttachmentPayload {
  kind: string;
  mime_type: string;
  data_base64: string;
  filename: string;
}

export interface SendMessageOutput {
  run_id: string;
}

export interface SettingsView {
  default_model_id: string | null;
  default_mode: string | null;
  egress_allowlist: string[];
  auto_update: boolean;
  telemetry_consent: boolean;
  language: string | null;
  theme: string | null;
}

export interface ToolView {
  id: string;
  name: string;
  description: string;
  builtin: boolean;
  default_permission: string;
  /** True when the default decision depends on the arguments (e.g. `files`). */
  mixed?: boolean;
  parameters: unknown;
}

export interface McpConnectionView {
  id: string;
  name: string;
  transport: string;
  enabled: boolean;
  status: string;
  tool_count: number;
}

export interface MemoryView {
  id: string;
  title: string;
  body: string;
  role: string;
  scope: string;
  lifecycle_state: string;
  verification: string;
  updated_at: string;
}

export interface SkillView {
  name: string;
  description: string;
  body_markdown: string;
  updated_at: string;
}

export interface ProjectView {
  id: string;
  name: string;
  instructions: string;
  rootPath: string | null;
  sources: string[];
  updated_at: string;
}

export interface ScheduleJobView {
  id: string;
  name: string;
  cron: string;
  enabled: boolean;
  last_run_at: string | null;
  next_run_at: string | null;
}

export interface AgentView {
  id: string;
  name: string;
  instructions: string;
  tool_ids: string[];
  updated_at: string;
}

export interface DiagnosticsReport {
  data_dir: string;
  db_ok: boolean;
  secrets_ok: boolean;
  engine_ok: boolean;
  provider_count: number;
  model_count: number;
  memory_count: number;
  skill_count: number;
  active_runs: number;
}

export interface CheckpointView {
  id: string;
  step_index: number;
  created_at: string;
}

export interface PermissionRuleView {
  id: string;
  tool_id: string;
  scope: string;
  decision: string;
}

export interface PendingApprovalView {
  approval_id: string;
  run_id: string;
  tool_call_id: string;
  tool_name: string;
  arguments: unknown;
  created_at: string;
  expires_at: string;
}

export interface RunView {
  id: string;
  conversation_id: string;
  mode: string;
  status: string;
  model_id: string;
  step_count: number;
  tool_call_count: number;
  started_at: string;
  updated_at: string;
  summary: string;
  budget: unknown;
}

export interface BrowserStatus {
  backend: string;
  headless: boolean;
  executable: string | null;
  profiles: number;
  artifacts: number;
}

export interface BrowserProfileView {
  workspace_id: string;
  display_name: string;
  directory: string;
  provisioned: boolean;
}

export interface BrowserInvokeOutput {
  ok: boolean;
  payload: unknown;
  artifact_id: string | null;
  error: string | null;
}

export interface MemoryRevisionView {
  id: string;
  title: string;
  content: string;
  created_at: string;
}

export interface KnowledgeDocView {
  doc_id: string;
  filename: string;
  chunk_count: number;
  created_at: string;
}

export interface KnowledgeSearchHit {
  id: string;
  doc_id: string;
  filename: string;
  text: string;
  score: number;
}

export interface SkillResourceView {
  path: string;
  content: string;
}

export interface DirEntryView {
  name: string;
  is_dir: boolean;
  size: number;
  hidden: boolean;
}

export interface FileContentView {
  path: string;
  content: string;
  truncated: boolean;
  is_binary: boolean;
}

export interface GitStatusEntry {
  path: string;
  status: string;
}

export interface GitStatusView {
  available: boolean;
  branch: string | null;
  entries: GitStatusEntry[];
}

export interface GitDiffView {
  available: boolean;
  scope: string;
  unified_diff: string;
  truncated: boolean;
}

/** Payload of the `update_available` event and of `check_for_updates`. */
export interface UpdateNotice {
  version: string;
  current_version: string;
  notes: string;
  date: string | null;
}
