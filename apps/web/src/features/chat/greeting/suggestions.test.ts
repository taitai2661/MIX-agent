import { describe, expect, it } from "vitest";
import { suggestionsFor } from "./suggestions";
import { buildSignals } from "./signals";

function signals(overrides: Partial<Parameters<typeof buildSignals>[0]> = {}) {
  const now = overrides.now ?? new Date(2026, 5, 10, 14, 0);
  return buildSignals({
    now,
    conversationCount: 8,
    pinnedCount: 1,
    projectCount: 1,
    agentCount: 1,
    lastMessageAt: new Date(now.getTime() - 5 * 3_600_000).getTime(),
    lastVisitAt: 0,
    firstSeenTodayAt: 0,
    visits: 0,
    ...overrides,
  });
}

function sequence(values: number[]): () => number {
  let index = 0;
  return () => values[index++ % values.length];
}

describe("提案の生成", () => {
  it("既定で 3 つを返す", () => {
    expect(suggestionsFor(signals(), [], 3, () => 0)).toHaveLength(3);
  });

  it("同じ提案を重複して返さない", () => {
    const all = suggestionsFor(
      signals(),
      [],
      20,
      sequence([0.1, 0.9, 0.5, 0.3, 0.7]),
    );
    expect(new Set(all.map((item) => item.id)).size).toBe(all.length);
  });

  it("直近で使った提案を避ける", () => {
    const recent = suggestionsFor(signals(), [], 3, () => 0).map(
      (item) => item.id,
    );
    const next = suggestionsFor(signals(), recent, 3, () => 0);
    for (const item of next) expect(recent).not.toContain(item.id);
  });

  it("履歴が無い場合は作り続けられる", () => {
    const all = suggestionsFor(signals(), [], 20, sequence([0.2, 0.8])).map(
      (item) => item.id,
    );
    expect(suggestionsFor(signals(), all, 3, () => 0)).toHaveLength(3);
  });

  it("昨日の活動があれば継続の提案を候補に入れる", () => {
    const now = new Date(2026, 5, 10, 14, 0);
    const yesterday = new Date(now.getTime() - 30 * 3_600_000);
    const ids = suggestionsFor(
      signals({ now, lastMessageAt: yesterday.getTime() }),
      [],
      20,
      sequence([0, 0, 0]),
    ).map((item) => item.id);
    expect(ids).toContain("sg-resume");
  });

  it("会話がゼロなら最初の一言を提案する", () => {
    const now = new Date(2026, 5, 10, 14, 0);
    const ids = suggestionsFor(
      signals({ now, conversationCount: 0, lastMessageAt: null }),
      [],
      20,
      sequence([0, 0, 0]),
    ).map((item) => item.id);
    expect(ids).toContain("sg-first");
  });

  it("会話が多いなら整理の提案を候補に入れる", () => {
    const ids = suggestionsFor(
      signals({ conversationCount: 40 }),
      [],
      20,
      sequence([0, 0, 0]),
    ).map((item) => item.id);
    expect(ids).toContain("sg-tidy");
  });

  it("プロジェクトとアシスタントがある場合だけ専用提案を出す", () => {
    const withAll = suggestionsFor(
      signals({ projectCount: 2, agentCount: 3 }),
      [],
      20,
      sequence([0, 0, 0]),
    ).map((item) => item.id);
    expect(withAll).toContain("sg-project");
    expect(withAll).toContain("sg-agent");

    const without = suggestionsFor(
      signals({ projectCount: 0, agentCount: 0 }),
      [],
      20,
      sequence([0, 0, 0]),
    ).map((item) => item.id);
    expect(without).not.toContain("sg-project");
    expect(without).not.toContain("sg-agent");
  });

  it("記録があっても会話がゼロでなければ最初の一言は提案しない", () => {
    const now = new Date(2026, 5, 10, 14, 0);
    const ids = suggestionsFor(
      signals({ now, conversationCount: 40, lastMessageAt: null }),
      [],
      20,
      sequence([0, 0, 0]),
    ).map((item) => item.id);
    expect(ids).not.toContain("sg-first");
  });

  it("夜には作業ログの提案を出す", () => {
    const night = new Date(2026, 5, 10, 23, 0);
    const ids = suggestionsFor(
      signals({ now: night }),
      [],
      20,
      sequence([0, 0, 0]),
    ).map((item) => item.id);
    expect(ids).toContain("sg-daylog");
  });

  it("調査や定期実行など実在の機能を提案する", () => {
    const ids = suggestionsFor(signals(), [], 20, sequence([0.4, 0.6])).map(
      (item) => item.id,
    );
    expect(ids).toContain("sg-research");
    expect(ids).toContain("sg-memory");
    expect(ids).toContain("sg-schedule");
  });

  it("ラベルとプロンプトが空でない", () => {
    for (const item of suggestionsFor(
      signals(),
      [],
      20,
      sequence([0.4, 0.6]),
    )) {
      expect(item.label.length, item.id).toBeGreaterThan(2);
      expect(item.prompt.length, item.id).toBeGreaterThan(8);
    }
  });
});
