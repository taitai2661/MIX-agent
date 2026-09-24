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
});
