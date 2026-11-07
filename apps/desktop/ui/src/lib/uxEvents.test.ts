import { beforeEach, describe, expect, it } from "vitest";
import {
  clearUxEvents,
  readUxEvents,
  recordUxEvent,
  UX_EVENTS_KEY,
} from "./uxEvents";

beforeEach(() => {
  localStorage.clear();
});

describe("uxEvents", () => {
  it("counts each event and keeps the most recent detail", () => {
    recordUxEvent("message_sent", { surface: "chat" });
    recordUxEvent("message_sent", { surface: "desktop-agent", tools: true });

    const events = readUxEvents();
    expect(events).toHaveLength(1);
    expect(events[0]).toMatchObject({
      name: "message_sent",
      count: 2,
      detail: { surface: "desktop-agent", tools: true },
    });
    expect(events[0]!.first_at <= events[0]!.last_at).toBe(true);
  });

  it("keeps events separate and sorts by most recent activity", () => {
    recordUxEvent("new_thread", { surface: "chat" });
    recordUxEvent("approval_decided", { location: "drawer", grant: true });

    const names = readUxEvents().map((event) => event.name);
    expect(names).toContain("new_thread");
    expect(names).toContain("approval_decided");
  });

  it("clears the aggregate", () => {
    recordUxEvent("search_opened", { location: "palette" });
    expect(readUxEvents()).toHaveLength(1);
    clearUxEvents();
    expect(readUxEvents()).toEqual([]);
    expect(localStorage.getItem(UX_EVENTS_KEY)).toBeNull();
  });

  it("ignores corrupt stored data instead of throwing", () => {
    localStorage.setItem(UX_EVENTS_KEY, "not-json");
    expect(readUxEvents()).toEqual([]);
  });
});
