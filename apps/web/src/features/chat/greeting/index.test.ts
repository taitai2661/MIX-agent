import { afterEach, describe, expect, it } from "vitest";
import { setLanguage } from "@/app/i18n";
import { pickWelcome, welcomeTitle, welcomeFrom } from "./index";
import { buildSignals } from "./signals";
import { emptyHistory, readHistory, remember, writeHistory, type WelcomeHistory } from "./history";

afterEach(() => setLanguage("ja"));

const now = new Date(2026, 5, 10, 20, 0);
const workspace = {
  conversationCount: 12,
  pinnedCount: 2,
  projectCount: 1,
  agentCount: 1,
  lastMessageAt: new Date(now.getTime() - 3_600_000).getTime(),
};

/** localStorage の代わりに使う、保存処理が例外を投げるメモリ実装。 */
function fakeStorage(value: string | null = null, fail = false): Storage {
  const store = new Map<string, string>();
  if (value !== null) store.set("mix-agent-welcome", value);
  return {
    get length() {
      return store.size;
    },
    clear: () => store.clear(),
    getItem: (key: string) => {
      if (fail) throw new Error("denied");
      return store.get(key) ?? null;
    },
    key: (index: number) => [...store.keys()][index] ?? null,
    removeItem: (key: string) => void store.delete(key),
    setItem: (key: string, item: string) => {
      if (fail) throw new Error("denied");
      store.set(key, item);
    },
  };
}

describe("訪問履歴の保存", () => {
  it("空の状態を既定にする", () => {
    expect(readHistory(fakeStorage())).toEqual({ visits: 0, lastVisitAt: 0, firstSeenTodayAt: 0, recent: [] });
    expect(readHistory(null)).toEqual(emptyHistory());
  });

  it("壊れた JSON でも既定に戻る", () => {
    expect(readHistory(fakeStorage("{ not json"))).toEqual(emptyHistory());
    expect(readHistory(fakeStorage('"string"'))).toEqual(emptyHistory());
  });

  it("保存と読み出しが一致する", () => {
    const store = fakeStorage();
    const history: WelcomeHistory = { visits: 3, lastVisitAt: 100, firstSeenTodayAt: 50, recent: ["a", "b"] };
    writeHistory(history, store);
    expect(readHistory(store)).toEqual(history);
  });

  it("例外を投げるストレージでも読み書きを続けられる", () => {
    const store = fakeStorage(null, true);
    expect(readHistory(store)).toEqual(emptyHistory());
    expect(() => writeHistory(emptyHistory(), store)).not.toThrow();
  });

  it("不正な型を安全に捨てる", () => {
    const store = fakeStorage(
      JSON.stringify({ visits: "many", lastVisitAt: null, firstSeenTodayAt: [], recent: ["ok", 3, null] }),
    );
    expect(readHistory(store)).toEqual({ visits: 0, lastVisitAt: 0, firstSeenTodayAt: 0, recent: ["ok"] });
  });

  it("使うたびに先頭に足し、長さ 24 で頭打ちにする", () => {
    let history = emptyHistory();
    for (let i = 0; i < 40; i++) history = remember(history, [`id-${i}`]);
    expect(history.recent).toHaveLength(24);
    expect(history.recent[0]).toBe("id-39");
    expect(history.recent).toContain("id-16");
    expect(history.recent).not.toContain("id-15");
  });
});

describe("文面の組み立て", () => {
  it("見出し・補足・提案が揃う", () => {
    const welcome = pickWelcome(workspace, { recent: [] }, () => 0, now);
    expect(welcome.title.length).toBeGreaterThanOrEqual(2);
    expect(welcome.note.length).toBeGreaterThan(4);
    expect(welcome.suggestions).toHaveLength(3);
    expect(welcome.id).toContain("/");
  });

  it("同じ乱数では同じ文面になる", () => {
    const first = pickWelcome(workspace, { recent: [] }, () => 0.5, now);
    const second = pickWelcome(workspace, { recent: [] }, () => 0.5, now);
    expect(second.title).toEqual(first.title);
    expect(second.note).toBe(first.note);
  });

  it("履歴の部品を避けて別の文面を作る", () => {
    const first = pickWelcome(workspace, { recent: [] }, () => 0.5, now);
    const second = pickWelcome(workspace, { recent: first.used }, () => 0.5, now);
    expect(second.id).not.toBe(first.id);
    expect(second.used.filter((id) => first.used.includes(id))).toEqual([]);
  });

  it("履歴が無い場合は作り続けられる", () => {
    const first = pickWelcome(workspace, { recent: [] }, () => 0.5, now);
    let recent = first.used;
    for (let i = 0; i < 30; i++) recent = [...pickWelcome(workspace, { recent }, () => 0.5, now).used, ...recent];
    const welcome = pickWelcome(workspace, { recent }, () => 0.5, now);
    expect(welcome.title.length).toBeGreaterThanOrEqual(2);
    expect(welcome.suggestions).toHaveLength(3);
  });

  it("見出しは日本語の文として終る", () => {
    for (let i = 0; i < 200; i++) {
      const welcome = pickWelcome(workspace, { recent: [] }, () => i / 200, now);
      expect(welcome.title.join("")).toMatch(/[。？]$/);
      expect(welcome.title.join("")).not.toMatch(/？{2,}/);
    }
  });

  it("祝日の朝はその祝日の文を含む", () => {
    // 2026年5月5日はこどもの日で、朝。
    const welcome = welcomeFrom(
      buildSignals({
        now: new Date(2026, 4, 5, 9, 0),
        conversationCount: 0,
        pinnedCount: 0,
        projectCount: 0,
        agentCount: 0,
        lastMessageAt: null,
        lastVisitAt: new Date(2026, 4, 3, 9, 0).getTime(),
        firstSeenTodayAt: new Date(2026, 4, 3, 9, 0).getTime(),
        visits: 2,
      }),
      [],
      () => 0.1,
    );
    expect(welcome.title.join("")).toContain("こどもの日です。");
  });

  it("英語表示では日本の祝日の文を出さない", () => {
    setLanguage("en");
    const holidaySignals = buildSignals({
      now: new Date(2026, 4, 5, 9, 0),
      conversationCount: 0,
      pinnedCount: 0,
      projectCount: 0,
      agentCount: 0,
      lastMessageAt: null,
      lastVisitAt: new Date(2026, 4, 3, 9, 0).getTime(),
      firstSeenTodayAt: new Date(2026, 4, 3, 9, 0).getTime(),
      visits: 2,
    });
    for (let i = 0; i < 20; i++) {
      const welcome = welcomeFrom(holidaySignals, [], () => i / 20);
      expect(welcome.titleIds).not.toContain("hl-9"); // こどもの日
      expect(welcome.title.join("")).not.toContain("こどもの日です。");
    }
  });
});

describe("言語ごとの結合", () => {
  it("日本語は間なしで連結する", () => {
    setLanguage("ja");
    const welcome = pickWelcome(workspace, { recent: [] }, () => 0, now);
    expect(welcomeTitle(welcome)).toBe(welcome.title.join(""));
  });

  it("英語は空白区切りで連結し、各文が終止符を持つ", () => {
    setLanguage("en");
    const welcome = pickWelcome(workspace, { recent: [] }, () => 0, now);
    const joined = welcomeTitle(welcome);
    expect(joined).toContain(" ");
    expect(joined).not.toContain(welcome.title[0]);
    expect(joined.split(/[.?!] /).length).toBeGreaterThanOrEqual(2);
    for (const sentence of joined.split(/(?<=[.?!]) /)) expect(sentence).toMatch(/[.?!]$/);
  });

  it("言語を切り替えると見出しだけが変わり、部品は同じ", () => {
    setLanguage("ja");
    const welcome = pickWelcome(workspace, { recent: [] }, () => 0, now);
    const japanese = welcomeTitle(welcome);
    setLanguage("en");
    expect(welcomeTitle(welcome).split(" ").length).toBeGreaterThanOrEqual(2);
    setLanguage("ja");
    expect(welcomeTitle(welcome)).toBe(japanese);
  });
});
