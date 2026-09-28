import { describe, expect, it } from "vitest";
import {
  applyRunEvent, createTimeline, describeToolCall, isFileWrite, isTerminal, reduceRunEvents,
} from "./runEvents";
import type { RunEventPayload } from "./api";

function event(kind: string, data: unknown, runId = "run-1", sequence = 1): RunEventPayload {
  return { run_id: runId, sequence, kind, data, created_at: "2026-01-01T00:00:00Z" };
}

describe("applyRunEvent", () => {
  it("ignores events from other runs", () => {
    const timeline = createTimeline("run-1");
    const next = applyRunEvent(timeline, event("text", { delta: "hi" }, "run-2"));
    expect(next.text).toBe("");
    expect(next).toBe(timeline);
  });

  it("accumulates text and reasoning deltas", () => {
    let timeline = createTimeline("run-1");
    timeline = applyRunEvent(timeline, event("text", { delta: "こん" }));
    timeline = applyRunEvent(timeline, event("text", { delta: "にちは" }, "run-1", 2));
    timeline = applyRunEvent(timeline, event("reasoning", { delta: "思考" }, "run-1", 3));
    expect(timeline.text).toBe("こんにちは");
    expect(timeline.reasoning).toBe("思考");
  });

  it("tolerates malformed payloads without throwing", () => {
    let timeline = createTimeline("run-1");
    timeline = applyRunEvent(timeline, event("text", null));
    timeline = applyRunEvent(timeline, event("status", "not-an-object"));
    timeline = applyRunEvent(timeline, event("tool_started", 42));
    expect(timeline.text).toBe("");
    expect(timeline.tools).toHaveLength(0);
  });

  it("tracks tool start and result by tool_call_id", () => {
    let timeline = createTimeline("run-1");
    timeline = applyRunEvent(timeline, event("tool_started", {
      tool_call_id: "tc-1", name: "files", arguments: { action: "read", path: "a.txt" },
    }));
    expect(timeline.tools).toHaveLength(1);
    expect(timeline.tools[0].status).toBe("running");

    timeline = applyRunEvent(timeline, event("tool_result", {
      tool_call_id: "tc-1", name: "files", result: { content: "x" },
    }, "run-1", 2));
    expect(timeline.tools[0].status).toBe("done");
    expect(timeline.tools[0].result).toEqual({ content: "x" });
    expect(timeline.tools[0].finishedAt).toBe("2026-01-01T00:00:00Z");
  });

  it("marks failed tool results and denials as errors", () => {
    let timeline = createTimeline("run-1");
    timeline = applyRunEvent(timeline, event("tool_started", {
      tool_call_id: "tc-1", name: "terminal", arguments: {},
    }));
    timeline = applyRunEvent(timeline, event("tool_result", {
      tool_call_id: "tc-1", name: "terminal", error: "exit 1",
    }, "run-1", 2));
    expect(timeline.tools[0].status).toBe("error");
    expect(timeline.tools[0].error).toBe("exit 1");

    timeline = applyRunEvent(timeline, event("tool_denied", {
      tool_call_id: "tc-1", reason: "user denied",
    }, "run-1", 3));
    expect(timeline.tools[0].status).toBe("error");
    expect(timeline.tools[0].error).toBe("user denied");
  });

  it("accepts a tool_result without a matching tool_started", () => {
    const timeline = applyRunEvent(
      createTimeline("run-1"),
      event("tool_result", { tool_call_id: "tc-x", name: "network", result: 1 }),
    );
    expect(timeline.tools).toHaveLength(1);
    expect(timeline.tools[0].status).toBe("done");
  });

  it("tracks the pending approval until it is resolved", () => {
    let timeline = createTimeline("run-1");
    timeline = applyRunEvent(timeline, event("tool_approval_requested", {
      tool_call_id: "tc-1", approval_id: "ap-1", name: "terminal", arguments: { command: "rm" },
    }));
    expect(timeline.approval).toEqual({
      approvalId: "ap-1", toolCallId: "tc-1", name: "terminal", arguments: { command: "rm" },
    });

    timeline = applyRunEvent(timeline, event("tool_approval_resolved", { granted: true }, "run-1", 2));
    expect(timeline.approval).toBeNull();
  });

  it("handles budget request lifecycle", () => {
    let timeline = createTimeline("run-1");
    timeline = applyRunEvent(timeline, event("budget_extension_requested", {
      reason: "steps", stepLimit: 300, toolCallLimit: 750,
    }));
    expect(timeline.budgetRequest).toEqual({ reason: "steps", stepLimit: 300, toolCallLimit: 750 });

    timeline = applyRunEvent(timeline, event("budget_extension_resolved", { granted: false }, "run-1", 2));
    expect(timeline.budgetRequest).toBeNull();
  });

  it("collects unique checkpoints and banners", () => {
    let timeline = createTimeline("run-1");
    timeline = applyRunEvent(timeline, event("checkpoint_saved", { checkpoint_id: "cp-1", step: 3 }));
    timeline = applyRunEvent(timeline, event("checkpoint_saved", { checkpoint_id: "cp-1", step: 3 }, "run-1", 2));
    timeline = applyRunEvent(timeline, event("stagnation_detected", { hint: "read loop" }, "run-1", 3));
    timeline = applyRunEvent(timeline, event("verification_required", { tool: "files" }, "run-1", 4));
    expect(timeline.checkpoints).toEqual(["cp-1"]);
    expect(timeline.stagnation).toBe("read loop");
    expect(timeline.verification).toEqual({ tool: "files", reason: "", outstanding: [] });
  });

  it("carries the reason and outstanding list on a verification request", () => {
    const timeline = applyRunEvent(createTimeline("run-1"), event("verification_required", {
      tool: "edit",
      reason: "edit changed a file on disk",
      outstanding: ["edit"],
    }));
    expect(timeline.verification).toEqual({
      tool: "edit",
      reason: "edit changed a file on disk",
      outstanding: ["edit"],
    });
  });

  it("records a failed verification separately from a pending one", () => {
    let timeline = createTimeline("run-1");
    timeline = applyRunEvent(timeline, event("destructive_tool_called", { tool: "edit", reason: "wrote" }));
    expect(timeline.verification?.tool).toBe("edit");
    expect(timeline.verificationFailed).toBeNull();

    timeline = applyRunEvent(timeline, event("verification_failed", { reason: "never read back" }));
    expect(timeline.verificationFailed).toBe("never read back");
  });

  it("tracks the agent checklist from todo_list events", () => {
    const timeline = applyRunEvent(createTimeline("run-1"), event("todo_list", {
      todos: [
        { id: 1, content: "read the code", status: "completed" },
        { id: 2, content: "write the fix", status: "in_progress" },
      ],
    }));
    expect(timeline.todos).toEqual([
      { id: 1, content: "read the code", status: "completed" },
      { id: 2, content: "write the fix", status: "in_progress" },
    ]);
  });

  it("drops malformed checklist entries rather than rendering blanks", () => {
    const timeline = applyRunEvent(createTimeline("run-1"), event("todo_list", {
      todos: [
        { id: 1, content: "good", status: "pending" },
        { id: "2", content: "bad id", status: "pending" },
        { content: "no id", status: "pending" },
        { id: 3, status: "pending" },
        "not an object",
      ],
    }));
    expect(timeline.todos).toEqual([{ id: 1, content: "good", status: "pending" }]);
  });

  it("recognises a targeted edit as a previewable write", () => {
    const write = {
      id: "c1", name: "edit", status: "done" as const,
      arguments: { path: "src/a.rs", old_string: "x", new_string: "y" },
    };
    const read = {
      id: "c2", name: "edit", status: "done" as const,
      arguments: { path: "src/a.rs" },
    };
    expect(isFileWrite(write)).toBe(true);
    expect(isFileWrite(read)).toBe(false);
  });

  it("labels the calls a coding agent makes most often", () => {
    const make = (name: string, args: unknown) => ({ id: "x", name, status: "done" as const, arguments: args });
    expect(describeToolCall(make("glob", { pattern: "**/*.rs" }))).toBe("**/*.rs");
    expect(describeToolCall(make("grep", { pattern: "fn main", path: "src" }))).toBe("fn main in src");
    expect(describeToolCall(make("files", { action: "read", path: "a.rs" }))).toBe("read · a.rs");
    expect(describeToolCall(make("edit", { action: "replace", path: "a.rs" }))).toBe("replace · a.rs");
    expect(describeToolCall(make("terminal", { command: "cargo", args: ["test"] }))).toBe("cargo test");
    expect(describeToolCall(make("skill", {}))).toBe("list skills");
    expect(describeToolCall(make("agent", { agent: "explorer", task: "find auth" }))).toBe("explorer: find auth");
  });

  it("ignores unknown event kinds", () => {
    const timeline = applyRunEvent(createTimeline("run-1"), event("future_kind", { x: 1 }));
    expect(timeline).toEqual(createTimeline("run-1"));
  });
});

describe("reduceRunEvents", () => {
  it("replays a persisted history into the same shape as live", () => {
    const timeline = reduceRunEvents("run-1", [
      event("status", { status: "running" }),
      event("tool_started", { tool_call_id: "t", name: "web_search", arguments: { query: "x" } }, "run-1", 2),
      event("tool_result", { tool_call_id: "t", name: "web_search", result: [1] }, "run-1", 3),
      event("text", { delta: "答え" }, "run-1", 4),
      event("status", { status: "completed" }, "run-1", 5),
    ]);
    expect(timeline.status).toBe("completed");
    expect(timeline.tools).toHaveLength(1);
    expect(timeline.text).toBe("答え");
  });
});

describe("isTerminal", () => {
  it("matches terminal statuses only", () => {
    expect(isTerminal("completed")).toBe(true);
    expect(isTerminal("failed")).toBe(true);
    expect(isTerminal("cancelled")).toBe(true);
    expect(isTerminal("interrupted")).toBe(true);
    expect(isTerminal("running")).toBe(false);
    expect(isTerminal("queued")).toBe(false);
  });
});

describe("isFileWrite", () => {
  it("detects files write with path and content", () => {
    expect(isFileWrite({
      id: "1", name: "files", arguments: { action: "write", path: "a.txt", content: "x" }, status: "done",
    })).toBe(true);
    expect(isFileWrite({
      id: "1", name: "files", arguments: { action: "read", path: "a.txt" }, status: "done",
    })).toBe(false);
    expect(isFileWrite({
      id: "1", name: "terminal", arguments: { action: "write" }, status: "done",
    })).toBe(false);
  });
});
