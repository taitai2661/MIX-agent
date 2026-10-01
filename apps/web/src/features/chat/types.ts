import type { components } from "@/generated/api";
import type { ActivitySummary } from "./ToolActivity";

export type ChatMode = components["schemas"]["MessageInput"]["mode"];
export type RunStatus = components["schemas"]["RunView"]["status"];
export type ApprovalDecision = components["schemas"]["DecisionInput"]["decision"];
export type Artifact = components["schemas"]["ArtifactView"];
export type ConversationHistory = components["schemas"]["ConversationMessagesView"];
export type Run = components["schemas"]["RunView"] & { browser_manual_active?: boolean; browser_pause_requested?: boolean };
export type Approval = components["schemas"]["ApprovalView"];
export type Tool = components["schemas"]["ToolView"];
export type PermissionRule = components["schemas"]["PermissionRuleView"];
export type SendMessage = components["schemas"]["SendMessageView"];
export type ToolCallHistory = components["schemas"]["ToolCallHistoryView"];
export type RunCheckpoint = components["schemas"]["RunCheckpointView"];
export type BudgetExtensionRequest = components["schemas"]["BudgetExtensionRequestView"];
export type StagnationFinding = {
  code: string;
  severity?: string;
  tool_id?: string;
  count?: number;
};

type JsonRecord = Record<string, unknown>;
const runStatuses = new Set<RunStatus>([
  "queued", "running", "waiting_approval", "paused", "budget_extension_pending",
  "completed", "failed", "cancelled", "interrupted",
]);
const eventKinds = new Set([
  "text", "reasoning", "model_started", "model_selected", "model_rerouted", "plan",
  "status", "tool_started", "tool_result", "approval", "message", "context_summary",
  "browser_frame", "checkpoint_saved", "budget_extension_requested",
  "budget_extension_resolved", "stagnation_detected", "verification_required",
  "phase_advanced",
]);

export type RunEvent = {
  kind: "text" | "reasoning";
  text: string;
} | {
  kind: "status";
  status: RunStatus;
  reason?: string;
} | {
  kind: "tool_started" | "tool_result" | "plan";
  id?: string;
  name?: string;
  result?: unknown;
  activity?: ActivitySummary;
} | {
  kind: "browser_frame";
  artifact_id: string;
  url: string;
  tool: string;
} | {
  kind: "approval";
  id: string;
  tool: string;
  arguments: JsonRecord;
  scope: JsonRecord;
  risk?: string;
} | {
  kind: "model_started" | "model_selected" | "model_rerouted" | "message" | "context_summary";
  name?: string;
  result?: unknown;
} | {
  kind: "checkpoint_saved";
  id: string;
  step?: number;
  trigger?: string;
  tool_count?: number;
} | {
  kind: "budget_extension_requested";
  tool_calls_used: number;
  tool_calls_limit: number;
  steps_used: number;
  steps_limit: number;
  elapsed_seconds: number;
  max_seconds: number;
  extensions_used?: number;
  extensions_max?: number;
} | {
  kind: "budget_extension_resolved";
  grant: boolean;
  max_seconds?: number;
  max_steps?: number;
  max_tool_calls?: number;
} | {
  kind: "stagnation_detected";
  findings: StagnationFinding[];
  step?: number;
} | {
  kind: "verification_required" | "phase_advanced";
  reason?: string;
  phase?: string;
};

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseActivity(value: unknown): ActivitySummary | undefined {
  if (!isRecord(value) || typeof value.label !== "string") return undefined;
  const sources = Array.isArray(value.sources)
    ? value.sources.filter(
        (source): source is { host: string; url: string } =>
          isRecord(source) && typeof source.host === "string" && typeof source.url === "string",
      )
    : undefined;
  return {
    ...(typeof value.icon === "string" ? { icon: value.icon } : {}),
    label: value.label,
    ...(typeof value.detail === "string" ? { detail: value.detail } : {}),
    ...(sources ? { sources } : {}),
    ...(typeof value.remaining === "number" ? { remaining: value.remaining } : {}),
  };
}

/** Returns null for malformed or unsupported server-sent events. */
export function parseRunEvent(data: string): RunEvent | null {
  let value: unknown;
  try {
    value = JSON.parse(data);
  } catch {
    return null;
  }
  if (!isRecord(value) || typeof value.kind !== "string" || !eventKinds.has(value.kind)) return null;
  if ((value.kind === "text" || value.kind === "reasoning") && typeof value.text === "string") {
    return { kind: value.kind, text: value.text };
  }
  if (value.kind === "status" && typeof value.status === "string" && runStatuses.has(value.status as RunStatus)) {
    return { kind: "status", status: value.status as RunStatus, ...(typeof value.reason === "string" ? { reason: value.reason } : {}) };
  }
  if (value.kind === "approval" && typeof value.id === "string" && typeof value.tool === "string" && isRecord(value.arguments) && isRecord(value.scope)) {
    return { kind: "approval", id: value.id, tool: value.tool, arguments: value.arguments, scope: value.scope, ...(typeof value.risk === "string" ? { risk: value.risk } : {}) };
  }
  if (value.kind === "browser_frame" && typeof value.artifact_id === "string" && typeof value.url === "string" && typeof value.tool === "string") {
    return { kind: "browser_frame", artifact_id: value.artifact_id, url: value.url, tool: value.tool };
  }
  if (value.kind === "checkpoint_saved" && typeof value.id === "string") {
    return {
      kind: "checkpoint_saved",
      id: value.id,
      ...(typeof value.step === "number" ? { step: value.step } : {}),
      ...(typeof value.trigger === "string" ? { trigger: value.trigger } : {}),
      ...(typeof value.tool_count === "number" ? { tool_count: value.tool_count } : {}),
    };
  }
  if (value.kind === "budget_extension_requested") {
    return {
      kind: "budget_extension_requested",
      tool_calls_used: numberField(value, "tool_calls_used"),
      tool_calls_limit: numberField(value, "tool_calls_limit"),
      steps_used: numberField(value, "steps_used"),
      steps_limit: numberField(value, "steps_limit"),
      elapsed_seconds: numberField(value, "elapsed_seconds"),
      max_seconds: numberField(value, "max_seconds"),
      ...(typeof value.extensions_used === "number" ? { extensions_used: value.extensions_used } : {}),
      ...(typeof value.extensions_max === "number" ? { extensions_max: value.extensions_max } : {}),
    };
  }
  if (value.kind === "budget_extension_resolved") {
    return {
      kind: "budget_extension_resolved",
      grant: Boolean(value.grant),
      ...(typeof value.max_seconds === "number" ? { max_seconds: value.max_seconds } : {}),
      ...(typeof value.max_steps === "number" ? { max_steps: value.max_steps } : {}),
      ...(typeof value.max_tool_calls === "number" ? { max_tool_calls: value.max_tool_calls } : {}),
    };
  }
  if (value.kind === "stagnation_detected") {
    const findings = Array.isArray(value.findings)
      ? value.findings.filter((item): item is StagnationFinding => isRecord(item) && typeof item.code === "string")
      : [];
    return {
      kind: "stagnation_detected",
      findings,
      ...(typeof value.step === "number" ? { step: value.step } : {}),
    };
  }
  if (value.kind === "verification_required" || value.kind === "phase_advanced") {
    return {
      kind: value.kind,
      ...(typeof value.reason === "string" ? { reason: value.reason } : {}),
      ...(typeof value.phase === "string" ? { phase: value.phase } : {}),
    };
  }
  if (["tool_started", "tool_result", "plan"].includes(value.kind)) {
    const activity = parseActivity(value.activity);
    return { kind: value.kind as "tool_started" | "tool_result" | "plan", ...(typeof value.id === "string" ? { id: value.id } : {}), ...(typeof value.name === "string" ? { name: value.name } : {}), ...("result" in value ? { result: value.result } : {}), ...(activity ? { activity } : {}) };
  }
  if (["model_started", "model_selected", "model_rerouted", "message", "context_summary"].includes(value.kind)) {
    return { kind: value.kind as "model_started" | "model_selected" | "model_rerouted" | "message" | "context_summary", ...(typeof value.name === "string" ? { name: value.name } : {}), ...("result" in value ? { result: value.result } : {}) };
  }
  return null;
}

function numberField(value: JsonRecord, key: string): number {
  const candidate = value[key];
  return typeof candidate === "number" ? candidate : 0;
}

/**
 * Derive the in-flight tool calls from the live SSE stream so the UI can show
 * progress before the persisted tool-call history is refetched. Raw results are
 * intentionally omitted; the redacted history replaces these entries by id.
 */
export function liveToolCalls(events: RunEvent[], runId = ""): ToolCallHistory[] {
  const order: string[] = [];
  const byId = new Map<string, ToolCallHistory>();
  const now = () => new Date().toISOString();
  for (const event of events) {
    if (event.kind === "tool_started") {
      const id = event.id || `live-${order.length}`;
      if (!byId.has(id)) order.push(id);
      byId.set(id, {
        id,
        run_id: runId,
        status: "running",
        tool_name: event.name || "Tool",
        activity: event.activity,
        created_at: now(),
        retry: { available: false, label: "" },
      });
    } else if (event.kind === "tool_result" || event.kind === "plan") {
      const id = event.id || order.at(-1) || `live-${order.length}`;
      const call = byId.get(id);
      const failed = isRecord(event.result) && Boolean(event.result.error);
      if (!call) order.push(id);
      byId.set(id, {
        id,
        run_id: runId,
        status: failed ? "failed" : "completed",
        tool_name: call?.tool_name || event.name || "Tool",
        activity: call?.activity,
        result_activity: event.activity,
        created_at: call?.created_at || now(),
        retry: { available: false, label: "" },
      });
    }
  }
  return order.flatMap((id) => {
    const call = byId.get(id);
    return call ? [call] : [];
  });
}

export function approvalFromEvent(event: Extract<RunEvent, { kind: "approval" }>): Approval {
  return {
    id: event.id,
    status: "pending",
    created_at: "",
    data: { tool: event.tool, arguments: event.arguments, scope: event.scope, ...(event.risk ? { risk: event.risk } : {}) },
  };
}
