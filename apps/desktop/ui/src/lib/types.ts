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
  auto_candidate: boolean;
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
  workspace_id: string | null;
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

/** Mirrors the Rust `SettingsView` one-for-one.
 *
 * Every key is listed even though the UI only edits a handful of them: the
 * backend replaces the stored document wholesale on every save, so a field the
 * type forgot to declare would be dropped from the row the next time the user
 * pressed Save anywhere in Settings.
 */
export interface SettingsView {
  default_model_id: string | null;
  default_mode: string | null;
  egress_allowlist: string[];
  auto_update: boolean;
  telemetry_consent: boolean;
  language: string | null;
  theme: string | null;
  projects_root: string | null;
  project_doc_max_bytes: number | null;
  project_doc_fallback_filenames: string[];
  /** Post-response memory auto formation (memory spec §6). Ships enabled. */
  memory_auto_formation: boolean;
  /**
   * §4 source (3): also write embedding-neighbour links after extraction.
   *
   * Ships **off** — §4 says 「(3) は既定オフ」 in as many words, because those
   * links grow with the square of the store. Never seed this `true`: a row read
   * before it arrives must render as "off", not as a switch the user never
   * threw.
   */
  memory_link_embedding_neighbors: boolean;
  /** App-wide default for the project's own `verify` gate (Rust `Option<String>`). */
  verify_command: string | null;
  /** App-wide default formatter/lint command reported to the model. */
  format_command: string | null;
  /** Refuse shell commands that write outside the workspace. */
  sandbox_terminal: boolean;
  /** Codex-compatible database encryption at rest. Defaults to false. */
  database_encryption: boolean;
  onboarding_completed: boolean | null;
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
  /** §1 `source_kind`: who or what wrote this row (`user` for direct input). */
  source_kind: string;
  lifecycle_state: string;
  verification: string;
  /** §1 `confidence`, 0..1 — the row's own certainty, not a search score. */
  confidence: number;
  /** §1 `salience`, 0..1 — how likely the row is to be recalled. */
  salience: number;
  /** §1 `strength`, 0..1 — how well the row is held; `decay` lowers it. */
  strength: number;
  /** §1 `pinned`: held — exempt from decay and always retrievable (§3). */
  pinned: boolean;
  /** §1 `superseded_by_id`: the Memory that replaced this one (§3). */
  superseded_by_id: string | null;
  /** §1 `disputed_by_id`: the Memory this one contradicts (§3). */
  disputed_by_id: string | null;
  /**
   * §4: how many `memory_links` edges touch this row, in either direction.
   *
   * **Absent when there are none** — the command omits the key rather than
   * sending `0`, so `undefined` here means "no edges", not "not loaded".
   */
  link_count?: number;
  updated_at: string;
}

/**
 * One row of `search_memories` (spec §7.2's `MemoryHit` plus §4's reason).
 *
 * Deliberately *not* `MemoryView`: the search path returns `content` (the §7.2
 * name) rather than `body`, carries `score`, and has none of §1's taxonomy
 * columns. Declaring it as `MemoryView` — which is what this used to do — made
 * `hit.body.slice(...)` type-check and then throw at runtime on the first
 * search.
 */
export interface MemorySearchHit {
  id: string;
  title: string;
  content: string;
  role: string;
  scope: string;
  /** §5's `score`. */
  score: number;
  /**
   * §4: `null` when the query matched this row, `"リンクから"` when the 1-hop
   * expansion pulled it in on an edge.
   */
  selection_reason: string | null;
}

/** What a settled dispute left behind (spec §3 `resolve_conflict`). */
export interface ConflictResolutionView {
  /** The adopted Memory, back to `active`. */
  winner: string;
  /** The rejected Memory; §3 deletes neither side. */
  loser: string;
  /** `superseded`, or `expired` when the resolution dismissed the pair. */
  loser_state: string;
  /** The resolution string sent through, echoed back. */
  resolution: string;
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
  /** Absolute folder the project is rooted at — every project has one. */
  rootPath: string;
  /** Read-only extra folders the agent may read from (Codex `--add-dir`). */
  additionalPaths: string[];
  sources: string[];
  created_at: string;
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
  /// `true` when the Keychain already holds a DB master key,
  /// `false` when one will be minted at first use,
  /// `undefined` when the Keychain refused to answer.
  db_key_present?: boolean;
  /// True once the storage layer is encrypted at rest; always false until
  /// the SQLCipher migration lands.
  db_encrypted: boolean;
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
  pattern: string | null;
  workspace_id: string | null;
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
  error: string | null;
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

/**
 * One `memory_links` row (spec §4): a directed edge between two Memories,
 * unique on (source, target, kind).
 *
 * `target_memory_id` is the row that carries §3's column and
 * `source_memory_id` is the Memory that column names — so `supersedes` reads
 * "source supersedes target". The table this describes is `memory_links`; the
 * legacy `memory_associations` table §9 #8 complained about is gone.
 */
export interface MemoryLinkView {
  id: string;
  source_memory_id: string;
  target_memory_id: string;
  kind: string;
  weight: number;
  coactivation_count: number;
  updated_at: string;
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
