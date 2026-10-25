import { describe, expect, it } from "vitest";
import { greetingEn } from "./translations";
import { suggestionLabelId, suggestionPromptId } from "./translate";
import { suggestionsFor } from "./suggestions";
import { buildSignals } from "./signals";

function signals(overrides: Partial<Parameters<typeof buildSignals>[0]> = {}) {
  const now = overrides.now ?? new Date(2026, 5, 10, 14, 0);
  return buildSignals({
    now,
    conversationCount: 8,
    messageCount: 40,
    projectCount: 1,
    lastMessageAt: now.getTime() - 5 * 3_600_000,
    lastVisitAt: now.getTime() - 3 * 86_400_000,
    firstSeenTodayAt: now.getTime() - 86_400_000,
    visits: 30,
    surface: "chat",
    projectActive: true,
    ...overrides,
  });
}

function sequence(values: number[]): () => number {
  let index = 0;
  return () => values[index++ % values.length];
}

const all = (
  s: ReturnType<typeof signals>,
  random = sequence([0.1, 0.9, 0.5, 0.3, 0.7]),
) => suggestionsFor(s, [], 30, random).map((item) => item.id);

describe("提案の生成", () => {
  it("既定で 3 つを返す", () => {
    expect(suggestionsFor(signals(), [], 3, () => 0)).toHaveLength(3);
  });

  it("同じ提案を重複して返さない", () => {
    const ids = suggestionsFor(
      signals(),
      [],
      20,
      sequence([0.1, 0.9, 0.5, 0.3, 0.7]),
    ).map((item) => item.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("直近で使った提案を避ける", () => {
    const recent = suggestionsFor(signals(), [], 3, () => 0).map(
      (item) => item.id,
    );
    const next = suggestionsFor(signals(), recent, 3, () => 0);
    for (const item of next) expect(recent).not.toContain(item.id);
  });

  it("履歴を溜めても 3 枚を保つ", () => {
    let recent: string[] = [];
    for (let visit = 0; visit < 60; visit++) {
      const picked = suggestionsFor(signals(), recent, 3, sequence([0.2, 0.8]));
      expect(picked).toHaveLength(3);
      recent = [...picked.map((item) => item.id), ...recent];
    }
  });

  it("ラベルとプロンプトに英語訳がある", () => {
    for (const item of suggestionsFor(
      signals(),
      [],
      30,
      sequence([0.1, 0.9]),
    )) {
      expect(greetingEn[suggestionLabelId(item.id)], item.id).toBeTruthy();
      expect(greetingEn[suggestionPromptId(item.id)], item.id).toBeTruthy();
    }
  });

  it("chat サーフェスでは思考モードの提案を出す", () => {
    const ids = all(signals({ surface: "chat" }));
    expect(ids).toContain("sg-explain");
  });

  it("Desktop-Agent では思考モードの指定を外す", () => {
    const agent = signals({ surface: "desktop-agent" });
    const items = suggestionsFor(agent, [], 30, sequence([0, 0]));
    expect(
      items.find((item) => item.id === "sg-explain")?.mode,
    ).toBeUndefined();
  });

  it("Desktop-Agent  では差分・テスト・ブラウザの提案を出す", () => {
    const ids = all(signals({ surface: "desktop-agent" }));
    expect(ids).toContain("sg-diff");
    expect(ids).toContain("sg-tests");
    expect(ids).toContain("sg-browser");
  });

  it("chat では差分・テストの提案を出さない", () => {
    const ids = all(signals({ surface: "chat" }));
    expect(ids).not.toContain("sg-diff");
    expect(ids).not.toContain("sg-tests");
  });

  it("プロジェクトの有無で提案を切り替える", () => {
    expect(all(signals({ projectActive: true }))).toContain("sg-instructions");
    expect(all(signals({ projectActive: false }))).toContain("sg-project");
    expect(all(signals({ projectActive: false }))).not.toContain(
      "sg-instructions",
    );
  });

  it("昨日の活動があれば継続の提案を候補に入れる", () => {
    const now = new Date(2026, 5, 10, 14, 0);
    const ids = all(
      signals({
        now,
        lastMessageAt: new Date(now.getTime() - 30 * 3_600_000).getTime(),
      }),
    );
    expect(ids).toContain("sg-resume");
  });

  it("スレッドがゼロなら最初の一言を提案する", () => {
    expect(
      all(signals({ conversationCount: 0, lastMessageAt: null })),
    ).toContain("sg-first");
  });

  it("スレッドが多いなら整理の提案を候補に入れる", () => {
    expect(all(signals({ conversationCount: 40 }))).toContain("sg-tidy");
  });

  it("夜には作業ログの提案を出す", () => {
    expect(all(signals({ now: new Date(2026, 5, 10, 23, 0) }))).toContain(
      "sg-daylog",
    );
  });

  it("ラベルとプロンプトが空でない", () => {
    for (const item of suggestionsFor(
      signals(),
      [],
      30,
      sequence([0.1, 0.9]),
    )) {
      expect(item.label.length, item.id).toBeGreaterThan(2);
      expect(item.prompt.length, item.id).toBeGreaterThan(8);
    }
  });
});
