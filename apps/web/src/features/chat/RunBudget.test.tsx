import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { RunBudget, remainingSeconds } from "./RunBudget";
import type { Run } from "./types";

const run = (overrides: Partial<Run> = {}): Run =>
  ({
    id: "r",
    status: "running",
    steps: 1,
    tool_count: 0,
    mode: "chat",
    policy: {},
    budget: { max_seconds: 1200, max_steps: 12, max_tool_calls: 12 },
    remaining: { max_seconds: 1181, max_steps: 11, max_tool_calls: 12 },
    approvals: [],
    ...overrides,
  }) as Run;

describe("remainingSeconds", () => {
  it("discounts the wall-clock time since the last server read", () => {
    expect(remainingSeconds(100, true, false, 5_000, 0)).toBe(95);
  });
  it("keeps the server value once the run is no longer active", () => {
    expect(remainingSeconds(100, false, false, 5_000, 0)).toBe(100);
  });
  it("freezes while manual browser control pauses the run", () => {
    expect(remainingSeconds(100, true, true, 5_000, 0)).toBe(100);
  });
  it("never grows when the client clock is ahead of the fetch time", () => {
    expect(remainingSeconds(100, true, false, 0, 5_000)).toBe(100);
  });
  it("floors at zero and tolerates a missing budget", () => {
    expect(remainingSeconds(3, true, false, 5_000, 0)).toBe(0);
    expect(remainingSeconds(null, true, false, 5_000, 0)).toBeNull();
  });
});

describe("RunBudget", () => {
  it("renders the current remaining budget", () => {
    const html = renderToStaticMarkup(
      <RunBudget run={run()} active={false} fetchedAt={0} />,
    );
    expect(html).toContain("chat");
    expect(html).toContain("1181");
    expect(html).toContain("11");
    expect(html).toContain("12 Tool Call");
  });
});
