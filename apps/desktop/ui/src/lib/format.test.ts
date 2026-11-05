import { beforeAll, describe, expect, it } from "vitest";
import {
  formatBytes,
  formatContext,
  formatCost,
  formatDuration,
  formatTokens,
  relativeTime,
  resetTime,
  threadGroup,
} from "./format";
import { setLanguage } from "./i18n";

// relativeTime() routes through t(); jsdom reports en-US, so pin the display
// language to keep the Japanese expectations below deterministic.
beforeAll(() => setLanguage("ja"));

// Local-time construction keeps the tests independent of the machine timezone.
const NOW = new Date(2026, 0, 15, 12, 0, 0);
const minutesAgo = (n: number) =>
  new Date(NOW.getTime() - n * 60_000).toISOString();
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

describe("resetTime", () => {
  it("renders UTC reset boundaries in the requested zone with the UTC clock", () => {
    // 2026-01-01T00:00:00Z is 09:00 on the same day in Japan.
    const label = resetTime(1767225600, "Asia/Tokyo");
    expect(label).toContain("09:00");
    expect(label).toContain("Asia/Tokyo");
    expect(label).toContain("UTC 00:00");
  });

  it("rolls the calendar day forward when UTC is still the previous day", () => {
    // 2026-01-01T15:00:00Z is 2026-01-02 00:00 in Japan.
    const label = resetTime(1767279600, "Asia/Tokyo");
    expect(label).toContain("01/02");
    expect(label).toContain("UTC 15:00");
  });

  it("returns an empty label for an unknown reset time", () => {
    expect(resetTime(0)).toBe("");
    expect(resetTime(Number.NaN)).toBe("");
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

describe("formatContext", () => {
  it("shortens token counts", () => {
    expect(formatContext(128000)).toBe("128K");
    expect(formatContext(200000)).toBe("200K");
    expect(formatContext(1_000_000)).toBe("1M");
    expect(formatContext(1_500_000)).toBe("1.5M");
    expect(formatContext(512)).toBe("512");
  });

  it("renders an unknown window as a dash", () => {
    expect(formatContext(0)).toBe("—");
    expect(formatContext(Number.NaN)).toBe("—");
  });
});

describe("formatTokens", () => {
  it("follows the display language's compact units", () => {
    setLanguage("ja");
    expect(formatTokens(60_000)).toBe("6万");
    expect(formatTokens(0)).toBe("0");
    setLanguage("en");
    expect(formatTokens(60_000)).toBe("60K");
    expect(formatTokens(0)).toBe("0");
    setLanguage("ja");
  });
});

describe("formatCost", () => {
  it("keeps small estimates legible and unpriced models honest", () => {
    expect(formatCost(0)).toBe("$0.00");
    expect(formatCost(0.0042)).toBe("$0.0042");
    expect(formatCost(0.42)).toBe("$0.420");
    expect(formatCost(1.234)).toBe("$1.23");
    expect(formatCost(null)).toBe("—");
  });
});
