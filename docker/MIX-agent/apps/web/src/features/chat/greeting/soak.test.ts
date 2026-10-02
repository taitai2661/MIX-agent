import { describe, expect, it } from "vitest";
import { en } from "@/app/i18n";
import { welcomeFrom } from "./index";
import { buildSignals } from "./signals";

function signalsFor(now: Date, overrides = {}) {
  return buildSignals({
    now,
    conversationCount: 12,
    pinnedCount: 2,
    projectCount: 1,
    agentCount: 1,
    lastMessageAt: now.getTime() - 3_600_000,
    lastVisitAt: now.getTime() - 3 * 86_400_000,
    firstSeenTodayAt: now.getTime() - 86_400_000,
    visits: 40,
    ...overrides,
  });
}

describe("全日程を通した合成", () => {
  it("2 年分のすべての組み合わせで見出しと提案が壊れない", () => {
    let seed = 1;
    const random = () => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed / 2147483648;
    };
    let combinations = 0;
    const titles = new Set<string>();
    for (let day = 0; day < 730; day++) {
      for (const hour of [3, 6, 8, 10, 13, 15, 18, 20, 23]) {
        const now = new Date(2026, 0, 1 + day, hour, 17);
        const welcome = welcomeFrom(signalsFor(now), [], random);
        const title = welcome.title.join("");
        combinations++;
        titles.add(title);
        expect(title).not.toMatch(/[\\s\\u3000]/);
        expect(title.endsWith("？") || title.endsWith("。")).toBe(true);
        expect(title).not.toMatch(/？{2,}/);
        expect(title.split("？").length).toBeLessThanOrEqual(2);
        expect(welcome.suggestions).toHaveLength(3);
        expect(new Set(welcome.suggestions.map((item) => item.id)).size).toBe(3);
      }
    }
    console.log("組合せ:", combinations, " 異なる見出し:", titles.size);
    expect(combinations).toBe(6570);
    // 同じ見出しが返り続けないこと。
    expect(titles.size).toBeGreaterThan(500);
  }, 60_000);

  it("見出しの各文に英語訳がある", () => {
    let seed = 7;
    const random = () => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed / 2147483648;
    };
    for (let day = 0; day < 365; day++) {
      for (const hour of [9, 20]) {
        const welcome = welcomeFrom(signalsFor(new Date(2026, 0, 1 + day, hour, 3)), [], random);
        for (const sentence of welcome.title) expect(Object.keys(en), sentence).toContain(sentence);
        expect(Object.keys(en), welcome.note).toContain(welcome.note);
      }
    }
  }, 60_000);

  it("履歴を溜めても枯渇しない", () => {
    let recent: string[] = [];
    let seed = 99;
    const random = () => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed / 2147483648;
    };
    for (let visit = 0; visit < 200; visit++) {
      const now = new Date(2026, 0, 1 + visit, 10 + (visit % 12), 5);
      const welcome = welcomeFrom(signalsFor(now), recent, random);
      recent = [...welcome.used, ...recent].slice(0, 24);
      expect(welcome.title.length).toBeGreaterThanOrEqual(2);
      expect(welcome.suggestions).toHaveLength(3);
    }
  }, 60_000);
});