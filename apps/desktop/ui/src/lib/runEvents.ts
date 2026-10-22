/// Pure reducer for `run_event` payloads. Shared by the live Tauri event
/// stream and the persisted `list_run_events` history so both render through
/// exactly the same logic.

import type { RunEventPayload } from "./api";

export interface ToolActivity {
  id: string;
  name: string;
  arguments: unknown;
  status: "running" | "done" | "error";
  result?: unknown;
  error?: string;
  startedAt?: string;
  finishedAt?: string;
}

export interface PendingApproval {
  approvalId: string | null;
  toolCallId: string;
  name: string;
  arguments: unknown;
}

export interface BudgetRequest {
  reason: string;
  stepLimit?: number;
  toolCallLimit?: number;
}

export interface VerificationNotice {
  tool: string;
  reason: string;
  outstanding: string[];
}

export interface RunTimeline {
  runId: string;
  text: string;
  reasoning: string;
  tools: ToolActivity[];
  status: string;
  budgetRequest: BudgetRequest | null;
  checkpoints: string[];
  stagnation: string | null;
  verification: VerificationNotice | null;
  /** Set when the run gave up on verifying an unconfirmed change. */
  verificationFailed: string | null;
  approval: PendingApproval | null;
  /** Terminal error text when the Run failed, so the user sees why. */
  error: string | null;
  /** The agent's own checklist, most recent state. */
  todos: TodoItem[];
}

export interface TodoItem {
  id: number;
  content: string;
  status: "pending" | "in_progress" | "completed";
}

export function createTimeline(runId: string): RunTimeline {
  return {
    runId,
    text: "",
    reasoning: "",
    tools: [],
    status: "idle",
    budgetRequest: null,
    checkpoints: [],
    stagnation: null,
    verification: null,
    verificationFailed: null,
    approval: null,
    error: null,
    todos: [],
  };
}

export const TERMINAL_STATUSES = [
  "completed",
  "failed",
  "cancelled",
  "interrupted",
];

/** A Run that is still doing something, including the two states where it is
 * waiting on the user rather than on the model. */
export const BUSY_STATUSES = [
  "queued",
  "running",
  "waiting_approval",
  "budget_extension_pending",
];

export function isTerminal(status: string): boolean {
  return TERMINAL_STATUSES.includes(status);
}

/** Whether a Run is in flight.
 *
 * The composer, the transcript spinner and the run actions each used to
 * decide this with their own list, and they disagreed: the transcript counted
 * `idle` (which is not a RunStatus at all) and so could hang on "thinking…"
 * forever, while the composer ignored `waiting_approval` and let the user send
 * into a run that was blocked on an approval card. One list, one meaning.
 */
export function isBusy(status: string): boolean {
  return BUSY_STATUSES.includes(status);
}

/** Narrow `unknown` event/argument payloads to a plain record. Exported so
 *  renderers share this guard instead of repeating ad-hoc `typeof` checks
 *  (see `ActivityPanel`, `BrowserPane`, `McpPane`). */
export function record(data: unknown): Record<string, unknown> {
  return typeof data === "object" && data !== null && !Array.isArray(data)
    ? (data as Record<string, unknown>)
    : {};
}

/** Narrow `unknown` to a string. Shared with renderers (see `record`). */
export function optString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

/** Narrow `unknown` to a number. Shared with renderers (see `record`). */
export function optNumber(value: unknown): number | undefined {
  return typeof value === "number" ? value : undefined;
}

/** Fold one event into the timeline. Unknown kinds are ignored. */
export function applyRunEvent(
  timeline: RunTimeline,
  event: RunEventPayload,
): RunTimeline {
  if (event.run_id !== timeline.runId) return timeline;
  const next: RunTimeline = { ...timeline };
  const d = record(event.data);

  switch (event.kind) {
    case "text":
      next.text += typeof d.delta === "string" ? d.delta : "";
      break;
    case "reasoning":
      next.reasoning += typeof d.delta === "string" ? d.delta : "";
      break;
    case "status": {
      const status = optString(d.status);
      if (status) next.status = status;
      const err = optString(d.error);
      if (err) next.error = err;
      break;
    }
    case "error": {
      // The engine only emits `error` on a terminal failure, so this is
      // authoritative even if some text had already streamed.
      const err = optString(d.error);
      if (err) next.error = err;
      next.status = "failed";
      break;
    }
    case "tool_started": {
      const id = optString(d.tool_call_id) ?? "";
      if (id && !next.tools.some((tool) => tool.id === id)) {
        next.tools = [
          ...next.tools,
          {
            id,
            name: optString(d.name) ?? "tool",
            arguments: d.arguments,
            status: "running",
            startedAt: event.created_at,
          },
        ];
      }
      break;
    }
    case "tool_result": {
      const id = optString(d.tool_call_id) ?? "";
      // Drop phantom results with an empty id, mirroring `tool_started`:
      // otherwise an id-less result renders as an anonymous "tool" row.
      if (!id) break;
      const error = optString(d.error);
      const patch = {
        result: d.result,
        error,
        status: (error ? "error" : "done") as ToolActivity["status"],
        finishedAt: event.created_at,
      };
      const idx = next.tools.findIndex((tool) => tool.id === id);
      if (idx >= 0) {
        next.tools = next.tools.map((tool, i) =>
          i === idx ? { ...tool, ...patch, error } : tool,
        );
      } else {
        next.tools = [
          ...next.tools,
          {
            id,
            name: optString(d.name) ?? "tool",
            arguments: d.arguments,
            startedAt: event.created_at,
            ...patch,
          },
        ];
      }
      break;
    }
    case "tool_denied": {
      const id = optString(d.tool_call_id) ?? "";
      if (!id) break;
      const idx = next.tools.findIndex((tool) => tool.id === id);
      if (idx >= 0) {
        next.tools = next.tools.map((tool, i) =>
          i === idx
            ? {
                ...tool,
                status: "error" as const,
                error: optString(d.reason) ?? "denied",
              }
            : tool,
        );
      }
      break;
    }
    case "tool_approval_requested":
      next.approval = {
        approvalId: optString(d.approval_id) ?? null,
        toolCallId: optString(d.tool_call_id) ?? "",
        name: optString(d.name) ?? "tool",
        arguments: d.arguments,
      };
      break;
    case "tool_approval_resolved":
      next.approval = null;
      break;
    case "budget_extension_requested":
      next.budgetRequest = {
        reason: optString(d.reason) ?? "",
        stepLimit: optNumber(d.stepLimit),
        toolCallLimit: optNumber(d.toolCallLimit),
      };
      break;
    case "budget_extension_resolved":
      next.budgetRequest = null;
      break;
    case "checkpoint_saved": {
      const id = optString(d.checkpoint_id);
      if (id && !next.checkpoints.includes(id)) {
        next.checkpoints = [...next.checkpoints, id];
      }
      break;
    }
    case "stagnation_detected":
      next.stagnation = optString(d.hint) ?? "";
      break;
    // The engine sends two shapes: a note that a destructive call just happened,
    // and a request that the agent go back and read the effect before
    // concluding. The second is the one the user needs to see.
    case "destructive_tool_called":
      next.verification = {
        tool: optString(d.tool) ?? "tool",
        reason: optString(d.reason) ?? "",
        outstanding: [],
      };
      break;
    case "verification_required":
      next.verification = {
        tool: optString(d.tool) ?? "tool",
        reason: optString(d.reason) ?? "",
        outstanding: Array.isArray(d.outstanding)
          ? (d.outstanding as unknown[]).filter(
              (x): x is string => typeof x === "string",
            )
          : [],
      };
      break;
    case "verification_failed":
      next.verificationFailed =
        optString(d.reason) ?? "an unconfirmed change was left";
      break;
    case "todo_list": {
      const items = Array.isArray(d.todos) ? d.todos : [];
      next.todos = items
        .map((raw) => {
          const todo = record(raw);
          const id = typeof todo.id === "number" ? todo.id : null;
          const content = optString(todo.content);
          const status = optString(todo.status);
          if (id === null || content === undefined || !status) return null;
          return { id, content, status: status as TodoItem["status"] };
        })
        .filter((t): t is TodoItem => t !== null);
      break;
    }
    default:
      break;
  }
  return next;
}

/** Reduce a whole batch of events (history replay or a throttled flush). */
export function reduceRunEvents(
  runId: string,
  events: RunEventPayload[],
): RunTimeline {
  return events.reduce(
    (timeline, event) => applyRunEvent(timeline, event),
    createTimeline(runId),
  );
}

/** True when the tool is a file write that can be previewed as a diff. */
export function isFileWrite(tool: ToolActivity): boolean {
  const args = record(tool.arguments);
  if (tool.name === "files") {
    return (
      args.action === "write" &&
      typeof args.path === "string" &&
      typeof args.content === "string"
    );
  }
  // `edit` is now the primary way a change happens, so it needs the same
  // preview the whole-file write already had.
  if (tool.name === "edit") {
    return (
      typeof args.path === "string" &&
      typeof args.old_string === "string" &&
      typeof args.new_string === "string"
    );
  }
  return false;
}

/**
 * A short, human label for a tool call.
 *
 * The raw JSON arguments in the activity list are unreadable for anything but a
 * whole-file write, and the agent's most frequent calls — a search, a glob, a
 * one-line edit — are exactly the ones that need naming.
 */
export function describeToolCall(tool: ToolActivity): string {
  const args = record(tool.arguments);
  const str = (key: string): string | undefined =>
    typeof args[key] === "string" ? (args[key] as string) : undefined;
  switch (tool.name) {
    case "files":
      return [str("action"), str("path")].filter(Boolean).join(" · ");
    case "edit":
      return [str("action") ?? "replace", str("path")]
        .filter(Boolean)
        .join(" · ");
    case "glob":
      return str("pattern") ?? "glob";
    case "grep":
      return [str("pattern"), str("path")].filter(Boolean).join(" in ");
    case "terminal":
      return [
        str("command"),
        ...(Array.isArray(args.args)
          ? (args.args as unknown[]).map(String)
          : []),
      ]
        .filter(Boolean)
        .join(" ");
    case "web_search":
    case "web_fetch":
      return str("url") ?? str("query") ?? tool.name;
    case "skill":
      return str("name") ?? "list skills";
    case "agent":
      return (
        [str("agent"), str("task")?.slice(0, 60)].filter(Boolean).join(": ") ||
        "list agents"
      );
    case "todo":
      return "checklist";
    case "user_plan":
      return str("title") ?? "plan";
    default:
      return tool.name;
  }
}
