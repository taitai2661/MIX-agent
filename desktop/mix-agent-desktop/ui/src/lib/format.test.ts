import { describe, expect, it } from "vitest";
import { formatBytes, formatDuration, relativeTime, threadGroup } from "./format";

// Local-time construction keeps the tests independent of the machine timezone.
const NOW = new Date(2026, 0, 15, 12, 0, 0);
const minutesAgo = (n: number) => new Date(NOW.getTime() - n * 60_000).toISOString();
const local = (y: number, m: number, d: number, h = 12) =>
  new Date(y, m, d, h, 0, 0).toISOString();

describe("relativeTime", () => {
  it("labels recent, minutes, hours, yesterday, days, and dates", () => {
    expect(relativeTime(minutesAgo(0), NOW)).toBe("たった今");
    expect(relativeTime(minutesAgo(30), NOW)).toBe("30分前");
    expect(relativeTime(minutesAgo(4 * 60), NOW)).toBe("4時間前");
    expect(relativeTime(local(2026, 0, 14), NOW)).toBe("昨日");
    expect(relativeTime(local(2026, 0, 12), NOW)).toBe("3日前");
    // Older than a week falls back to a locale-formatted date.
    const old = relativeTime(local(2025, 11, 1), NOW);
    expect(old).not.toContain("前");
    expect(old.length).toBeGreaterThan(0);
  });

  it("returns empty for invalid input", () => {
    expect(relativeTime("not-a-date")).toBe("");
  });
});

describe("threadGroup", () => {
  it("buckets timestamps for the sidebar", () => {
    expect(threadGroup(local(2026, 0, 15, 8), NOW)).toBe("today");
    expect(threadGroup(local(2026, 0, 14, 23), NOW)).toBe("yesterday");
    expect(threadGroup(local(2026, 0, 10), NOW)).toBe("week");
    expect(threadGroup(local(2025, 11, 20), NOW)).toBe("older");
    expect(threadGroup("garbage", NOW)).toBe("older");
  });
});

describe("formatBytes", () => {
  it("formats byte magnitudes", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});

describe("formatDuration", () => {
  it("formats ms, seconds, and minutes", () => {
    expect(formatDuration(120)).toBe("120ms");
    expect(formatDuration(2500)).toBe("2.5s");
    expect(formatDuration(90_000)).toBe("1m 30s");
    expect(formatDuration(120_000)).toBe("2m");
  });
});
