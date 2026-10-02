import { describe, expect, it } from "vitest";
import { compatible, compose, contextCandidates, noteCandidates, openingCandidates, pickFragment } from "./compose";
import { openings } from "./pools";
import { buildSignals } from "./signals";
import type { Fragment, Signals } from "./types";

function signals(overrides: Partial<Parameters<typeof buildSignals>[0]> = {}): Signals {
  const now = overrides.now ?? new Date(2026, 5, 10, 14, 0);
  return buildSignals({
    now,
    conversationCount: 8,
    messageCount: 40,
    projectCount: 1,
    lastMessageAt: new Date(now.getTime() - 5 * 3_600_000).getTime(),
    lastVisitAt: now.getTime() - 3 * 86_400_000,
    firstSeenTodayAt: now.getTime() - 86_400_000,
    visits: 30,
    surface: "chat",
    projectActive: true,
    ...overrides,
  });
}

/** 決定的に回す擬似乱数。 */
function sequence(values: number[]): () => number {
  let index = 0;
  return () => values[index++ % values.length];
}

const question: Fragment = { id: "q", text: "EWFew？", tone: "polite", form: "question" };
const questionCasual: Fragment = { id: "q2", text: "EWFew？", tone: "casual", form: "question" };
const statement: Fragment = { id: "s", text: "EWFew。", tone: "polite", form: "declarative" };
const casual: Fragment = { id: "c", text: "EWFew。", tone: "casual", form: "declarative" };
const calm: Fragment = { id: "k", text: "EWFew。", tone: "calm", form: "declarative" };

describe("文体の互換判定", () => {
  it("疑問文のあとには疑問文をつなげない", () => {
    expect(compatible(question, null, statement)).toBe(true);
    expect(compatible(question, null, question)).toBe(false);
    expect(compatible(statement, null, question)).toBe(true);
  });

  it("文体が違う組み合わせは捨てる", () => {
    expect(compatible(statement, null, casual)).toBe(false);
    expect(compatible(statement, null, calm)).toBe(true);
    expect(compatible(casual, null, questionCasual)).toBe(true);
    expect(compatible(questionCasual, null, question)).toBe(false);
  });

  it("状況節も同じ判定を通す", () => {
    expect(compatible(statement, casual, statement)).toBe(false);
    expect(compatible(statement, calm, statement)).toBe(true);
  });
});

describe("部品の抽選", () => {
  it("直近で使った部品を除外する", () => {
    const pool = openings.morning;
    expect(pickFragment(pool, new Set(pool.map((fragment) => fragment.id)), () => 0.5)?.id).toBeDefined();
  });

  it("空のプールでは undefined を返す", () => {
    expect(pickFragment([], new Set(), () => 0)).toBeUndefined();
  });

  it("random が 0 でも 1 に近い値でも範囲外にならない", () => {
    const pool = openings.afternoon;
    expect(pool).toContain(pickFragment(pool, new Set(), () => 0));
    expect(pool).toContain(pickFragment(pool, new Set(), () => 0.999999));
    expect(pool).toContain(pickFragment(pool, new Set(), () => 1));
  });

  it("重みが大きい部品が選ばれやすい", () => {
    const pool: Fragment[] = [
      { ...statement, id: "rare", weight: 1 },
      { ...statement, id: "common", weight: 9 },
    ];
    let common = 0;
    for (let i = 0; i < 200; i++) if (pickFragment(pool, new Set(), () => i / 200)?.id === "common") common++;
    expect(common).toBeGreaterThan(150);
  });
});

describe("候補の組み立て", () => {
  it("初回ははじめましての文を候補に入れる", () => {
    expect(openingCandidates(signals({ lastVisitAt: 0 })).some((f) => f.id.startsWith("ft-"))).toBe(true);
  });

  it("初回ははじめましての文だけに絞る", () => {
    const first = openingCandidates(signals({ lastVisitAt: 0 }));
    expect(first.every((fragment) => fragment.id.startsWith("ft-"))).toBe(true);
  });

  it("再訪ならおかえりなさいの文を候補に入れる", () => {
    const later = new Date(2026, 5, 10, 14, 0);
    expect(
      openingCandidates(signals({ now: later, lastVisitAt: later.getTime() - 3 * 86_400_000 })).some((f) =>
        f.id.startsWith("rt-"),
      ),
    ).toBe(true);
  });

  it("リロードではおかえりなさいを出さない", () => {
    const now = new Date(2026, 5, 10, 14, 0);
    expect(openingCandidates(signals({ now, lastVisitAt: now.getTime() - 60_000 })).some((f) => f.id.startsWith("rt-"))).toBe(false);
  });

  it("祝日は状況節の候補の最優先になる", () => {
    const candidates = contextCandidates(signals({ now: new Date(2026, 4, 5, 15, 0) }));
    const holiday = candidates.find((f) => f.id === "hl-9");
    expect(holiday?.text).toBe("こどもの日です。");
    expect(candidates[0].weight).toBeGreaterThan(1);
  });

  it("プロジェクトの有無で状況節を切り替える", () => {
    const withProject = contextCandidates(signals({ projectActive: true }));
    const withoutProject = contextCandidates(signals({ projectActive: false }));
    expect(withProject.some((f) => f.id === "pc-1")).toBe(true);
    expect(withoutProject.some((f) => f.id === "np-1")).toBe(true);
    expect(withoutProject.some((f) => f.id === "pc-1")).toBe(false);
  });

  it("Desktop-Agent では実行モデルを訴える補足を選ぶ", () => {
    const agent = noteCandidates(signals({ surface: "desktop-agent" }));
    expect(agent.some((f) => f.id === "nt-17")).toBe(true);
    const chat = noteCandidates(signals({ surface: "chat" }));
    expect(chat.some((f) => f.id === "nt-17")).toBe(false);
  });

  it("補足は常に既定の文を含める", () => {
    expect(noteCandidates(signals()).some((f) => f.id === "nt-1")).toBe(true);
  });
});

describe("見出しの合成", () => {
  it("2 文または 3 文の文単位を返す", () => {
    for (let i = 0; i < 100; i++) {
      const composed = compose(signals(), [], () => i / 100);
      expect(composed.title.length).toBeGreaterThanOrEqual(2);
      expect(composed.title.length).toBeLessThanOrEqual(3);
      for (const part of composed.title) expect(part.length).toBeGreaterThan(4);
    }
  });

  it("疑問符が連続することがない", () => {
    for (let i = 0; i < 300; i++) {
      const { title } = compose(signals(), [], () => i / 300);
      expect(title.filter((part) => part.endsWith("？")).length).toBeLessThanOrEqual(1);
    }
  });

  it("同じ乱数では同じ結果になる", () => {
    const first = compose(signals(), [], sequence([0.1, 0.7, 0.3]));
    const second = compose(signals(), [], sequence([0.1, 0.7, 0.3]));
    expect(second.title).toEqual(first.title);
    expect(second.note.id).toBe(first.note.id);
  });

  it("直近で使っていない部品が選ばれる", () => {
    const { used } = compose(signals(), [], sequence([0, 0, 0]));
    const blocked = compose(signals(), used, sequence([0.5, 0.5, 0.5]));
    for (const id of blocked.used) expect(used).not.toContain(id);
  });

  it("直近の部品をすべて指定しても作り続けられる", () => {
    const all = [...openingCandidates(signals()), ...contextCandidates(signals()), ...noteCandidates(signals())].map(
      (fragment) => fragment.id,
    );
    const composed = compose(signals(), all, sequence([0.5, 0.5, 0.5]));
    expect(composed.title.length).toBeGreaterThanOrEqual(2);
  });

  it("時間帯に応じた挨拶を使う", () => {
    const night = new Date(2026, 5, 10, 23, 0);
    const sameDay = compose(
      signals({ now: night, lastVisitAt: night.getTime() - 3_600_000 }),
      [],
      sequence([0, 0, 0]),
    );
    expect(sameDay.title[0]).toContain("こんばんは。");
    expect(sameDay.title.join("")).not.toContain("おはよう");
  });

  it("しばらく空いた再訪ではおかえりなさいを使う", () => {
    const night = new Date(2026, 5, 10, 23, 0);
    const returning = compose(
      signals({ now: night, lastVisitAt: night.getTime() - 3 * 86_400_000 }),
      [],
      sequence([0, 0, 0]),
    );
    expect(returning.title[0]).toContain("おかえりなさい。");
  });

  it("初回は必ずはじめましてから始める", () => {
    for (let i = 0; i < 40; i++) {
      const first = compose(signals({ lastVisitAt: 0 }), [], () => i / 40);
      expect(first.title[0]).toContain("はじめまして");
    }
  });
});