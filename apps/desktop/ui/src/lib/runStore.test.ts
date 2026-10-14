/// Flush-timer behaviour of the live run-event store (`lib/runStore.ts`).
///
/// Deltas are buffered and applied on a 60ms timer so token streams cause at
/// most ~16 renders per second. These tests pin the batching contract: pushes
/// are invisible until the timer fires, foreign runs are ignored, and `clear`
/// / `startRun` drop pending work.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useRunEvents } from "./runStore";
import type { RunEventPayload } from "./api";

function event(
  kind: string,
  data: unknown,
  runId = "run-1",
  sequence = 1,
): RunEventPayload {
  return {
    run_id: runId,
    sequence,
    kind,
    data,
    created_at: "2026-01-01T00:00:00Z",
  };
}

describe("useRunEvents flush timer", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    useRunEvents.getState().clear();
    vi.clearAllTimers();
  });

  afterEach(() => {
    useRunEvents.getState().clear();
    vi.useRealTimers();
  });

  it("starts tracking a run with an empty timeline", () => {
    useRunEvents.getState().startRun("run-1", "conv-1");
    const state = useRunEvents.getState();
    expect(state.liveRunId).toBe("run-1");
    expect(state.conversationId).toBe("conv-1");
    expect(state.timeline?.text).toBe("");
  });

  it("buffers pushes and applies them when the flush timer fires", () => {
    useRunEvents.getState().startRun("run-1", "conv-1");
    useRunEvents.getState().push(event("text", { delta: "こん" }));
    useRunEvents
      .getState()
      .push(event("text", { delta: "にちは" }, "run-1", 2));
    // Nothing applied yet — the buffer only drains on the timer.
    expect(useRunEvents.getState().timeline?.text).toBe("");
    vi.advanceTimersByTime(60);
    expect(useRunEvents.getState().timeline?.text).toBe("こんにちは");
  });

  it("ignores events from other runs without scheduling a flush", () => {
    useRunEvents.getState().startRun("run-1", "conv-1");
    useRunEvents.getState().push(event("text", { delta: "hi" }, "run-2"));
    vi.advanceTimersByTime(1000);
    expect(useRunEvents.getState().timeline?.text).toBe("");
  });

  it("drops buffered events on clear", () => {
    useRunEvents.getState().startRun("run-1", "conv-1");
    useRunEvents.getState().push(event("text", { delta: "lost" }));
    useRunEvents.getState().clear();
    expect(useRunEvents.getState().liveRunId).toBeNull();
    vi.advanceTimersByTime(1000);
    // A stale flush must not resurrect the cleared timeline.
    expect(useRunEvents.getState().timeline).toBeNull();
  });

  it("startRun discards the previous run's buffer", () => {
    useRunEvents.getState().startRun("run-1", "conv-1");
    useRunEvents.getState().push(event("text", { delta: "stale" }));
    useRunEvents.getState().startRun("run-2", "conv-1");
    vi.advanceTimersByTime(1000);
    expect(useRunEvents.getState().timeline?.text).toBe("");
    expect(useRunEvents.getState().liveRunId).toBe("run-2");
  });
});
