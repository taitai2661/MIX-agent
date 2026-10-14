import { describe, expect, it } from "vitest";

import { parseRunEvent } from "./types";

describe("parseRunEvent", () => {
  it("accepts a valid activity event", () => {
    expect(parseRunEvent(JSON.stringify({
      kind: "tool_started",
      id: "call-1",
      name: "web_search",
      activity: { label: "Webを検索中", detail: "AI" },
    }))).toEqual({
      kind: "tool_started",
      id: "call-1",
      name: "web_search",
      activity: { label: "Webを検索中", detail: "AI" },
    });
  });

  it("ignores malformed, unknown, and incomplete events", () => {
    expect(parseRunEvent("not json")).toBeNull();
    expect(parseRunEvent(JSON.stringify({ kind: "future_event" }))).toBeNull();
    expect(parseRunEvent(JSON.stringify({ kind: "text" }))).toBeNull();
    expect(parseRunEvent(JSON.stringify({ kind: "status", status: "unknown" }))).toBeNull();
  });

  it("accepts browser frames without carrying manual input into UI state", () => {
    expect(parseRunEvent(JSON.stringify({
      kind: "browser_frame", artifact_id: "frame-1", url: "https://example.com", tool: "manual", text: "secret",
    }))).toEqual({ kind: "browser_frame", artifact_id: "frame-1", url: "https://example.com", tool: "manual" });
  });

  it("parses checkpoint_saved events", () => {
    expect(parseRunEvent(JSON.stringify({
      kind: "checkpoint_saved", id: "ck-1", step: 12, trigger: "interval", tool_count: 4,
    }))).toEqual({
      kind: "checkpoint_saved", id: "ck-1", step: 12, trigger: "interval", tool_count: 4,
    });
  });

  it("parses budget_extension_requested events", () => {
    expect(parseRunEvent(JSON.stringify({
      kind: "budget_extension_requested",
      tool_calls_used: 8, tool_calls_limit: 12,
      steps_used: 240, steps_limit: 300,
      elapsed_seconds: 4800, max_seconds: 5400,
      extensions_used: 1, extensions_max: 2,
    }))).toMatchObject({
      kind: "budget_extension_requested",
      tool_calls_used: 8,
      steps_used: 240,
      extensions_used: 1,
      extensions_max: 2,
    });
  });

  it("parses stagnation_detected events with findings", () => {
    expect(parseRunEvent(JSON.stringify({
      kind: "stagnation_detected",
      findings: [
        { code: "tool_repeated_failure", severity: "warn", tool_id: "read_file", count: 4 },
        { code: "stale_plan", severity: "info" },
      ],
      step: 18,
    }))).toEqual({
      kind: "stagnation_detected",
      findings: [
        { code: "tool_repeated_failure", severity: "warn", tool_id: "read_file", count: 4 },
        { code: "stale_plan", severity: "info" },
      ],
      step: 18,
    });
  });

  it("accepts budget_extension_pending as a known run status", () => {
    expect(parseRunEvent(JSON.stringify({
      kind: "status", status: "budget_extension_pending",
    }))).toMatchObject({ kind: "status", status: "budget_extension_pending" });
  });
});
