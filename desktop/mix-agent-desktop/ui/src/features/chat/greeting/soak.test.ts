import { beforeEach, describe, expect, it } from "vitest";
import { setLanguage } from "@/lib/i18n";
import { greetingEn } from "./translations";
import { welcomeFrom } from "./index";
import { buildSignals } from "./signals";

// 全日程の組み合わせを検証するため、日本の暦を含む日本語表示に固定する。
beforeEach(() => setLanguage("ja"));

function signalsFor(now: Date, overrides = {}) {
  return buildSignals({
    now,
    conversationCount: 12,
    messageCount: 140,
    projectCount: 1,
    lastMessageAt: now.getTime() - 3_600_000,
    lastVisitAt: now.getTime() - 3 * 86_400_000,
    firstSeenTodayAt: now.getTime() - 86_400_000,
    visits: 40,
    surface: "chat",
    projectActive: true,
    ...overrides,
  });
}

function seeded(seed: number): () => number {
  let state = seed;
  return () => {
    state = (state * 1103515245 + 12345) % 2147483648;
    return state / 2147483648;
  };
}

describe("全日程を通した合成", () => {
  it("2 年分のすべての組み合わせで見出しと提案が壊れない", () => {
    const random = seeded(1);
    let combinations = 0;
    const titles = new Set<string>();
    for (let day = 0; day < 730; day++) {
      for (const hour of [3, 6, 8, 10, 13, 15, 18, 20, 23]) {
        const now = new Date(2026, 0, 1 + day, hour, 17);
        const welcome = welcomeFrom(signalsFor(now), [], random);
        const title = welcome.title.join("");
        combinations++;
        titles.add(title);
        expect(title).toMatch(/[。？]$/);
        expect(title).not.toMatch(/？{2,}/);
        expect(title).not.toMatch(/[ \u3000]/);
        expect(welcome.suggestions).toHaveLength(3);
        expect(new Set(welcome.suggestions.map((item) => item.id)).size).toBe(3);
      }
    }
    console.log("組合せ:", combinations, " 異なる見出し:", titles.size);
    expect(combinations).toBe(6570);
    // 同じ見出しが返り続けないこと。
    expect(titles.size).toBeGreaterThan(500);
  }, 60_000);

  it("Desktop-Agent でも同様に壊れない", () => {
    const random = seeded(3);
    for (let day = 0; day < 365; day++) {
      const welcome = welcomeFrom(
        signalsFor(new Date(2026, 0, 1 + day, 21, 3), { surface: "desktop-agent" }),
        [],
        random,
      );
      expect(welcome.title.join("")).toMatch(/[。？]$/);
      expect(welcome.suggestions).toHaveLength(3);
      expect(welcome.suggestions.some((item) => item.mode === "thinking")).toBe(false);
    }
  }, 60_000);

  it("見出しの各文に英語訳がある", () => {
    const keys = greetingEn;
    const random = seeded(7);
    for (let day = 0; day < 365; day++) {
      for (const hour of [9, 20]) {
        const welcome = welcomeFrom(signalsFor(new Date(2026, 0, 1 + day, hour, 3)), [], random);
        for (const id of welcome.titleIds) expect(keys[id], id).toBeTruthy();
        expect(keys[welcome.noteId], welcome.noteId).toBeTruthy();
      }
    }
  }, 60_000);

  it("履歴を溜めても枯渇しない", () => {
    let recent: string[] = [];
    const random = seeded(99);
    for (let visit = 0; visit < 200; visit++) {
      const now = new Date(2026, 0, 1 + visit, 10 + (visit % 12), 5);
      const welcome = welcomeFrom(signalsFor(now), recent, random);
      recent = [...welcome.used, ...recent].slice(0, 24);
      expect(welcome.title.length).toBeGreaterThanOrEqual(2);
      expect(welcome.suggestions).toHaveLength(3);
    }
  }, 60_000);
});
