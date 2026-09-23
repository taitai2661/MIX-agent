import type { components } from "@/generated/api";
import type { ActivitySummary } from "./ToolActivity";

export type ChatMode = components["schemas"]["MessageInput"]["mode"];
export type RunStatus = components["schemas"]["RunView"]["status"];
export type ApprovalDecision = components["schemas"]["DecisionInput"]["decision"];
export type Artifact = components["schemas"]["ArtifactView"];
export type ConversationHistory = components["schemas"]["ConversationMessagesView"];
export type Run = components["schemas"]["RunView"];
export type Approval = components["schemas"]["ApprovalView"];
export type Tool = components["schemas"]["ToolView"];
export type PermissionRule = components["schemas"]["PermissionRuleView"];
export type SendMessage = components["schemas"]["SendMessageView"];
export type ToolCallHistory = components["schemas"]["ToolCallHistoryView"];

type JsonRecord = Record<string, unknown>;
const runStatuses = new Set<RunStatus>([
  "queued", "running", "waiting_approval", "completed", "failed", "cancelled", "interrupted",
]);
const eventKinds = new Set([
  "text", "reasoning", "model_started", "model_selected", "model_rerouted", "plan",
  "status", "tool_started", "tool_result", "approval", "message", "context_summary",
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
  if (["tool_started", "tool_result", "plan"].includes(value.kind)) {
    const activity = parseActivity(value.activity);
    return { kind: value.kind as "tool_started" | "tool_result" | "plan", ...(typeof value.id === "string" ? { id: value.id } : {}), ...(typeof value.name === "string" ? { name: value.name } : {}), ...("result" in value ? { result: value.result } : {}), ...(activity ? { activity } : {}) };
  }
  if (["model_started", "model_selected", "model_rerouted", "message", "context_summary"].includes(value.kind)) {
    return { kind: value.kind as "model_started" | "model_selected" | "model_rerouted" | "message" | "context_summary", ...(typeof value.name === "string" ? { name: value.name } : {}), ...("result" in value ? { result: value.result } : {}) };
  }
  return null;
}

export function approvalFromEvent(event: Extract<RunEvent, { kind: "approval" }>): Approval {
  return {
    id: event.id,
    status: "pending",
    created_at: "",
    data: { tool: event.tool, arguments: event.arguments, scope: event.scope, ...(event.risk ? { risk: event.risk } : {}) },
  };
}
