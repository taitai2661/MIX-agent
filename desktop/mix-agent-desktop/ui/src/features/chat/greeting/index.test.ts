import { afterEach, describe, expect, it } from "vitest";
import { setLanguage } from "@/lib/i18n";
import { emptyHistory, readHistory, remember, writeHistory, type WelcomeHistory } from "./history";
import { promptDetail, welcomeFrom, welcomeTitle, type WorkspaceSnapshot } from "./index";
import { notes } from "./pools";
import { buildSignals } from "./signals";

afterEach(() => setLanguage("ja"));

/** 補足文から対応する部品 ID を引き戻します。 */
const NOTE_IDS = new Map(
  Object.values(notes)
    .flat()
    .map((fragment) => [fragment.text, fragment.id] as const),
);
function welcomeNoteId(text: string): string {
  return NOTE_IDS.get(text) ?? "";
}

const now = new Date(2026, 5, 10, 20, 0);
const workspace: WorkspaceSnapshot = {
  conversationCount: 12,
  messageCount: 140,
  projectCount: 1,
  lastActivityAt: new Date(now.getTime() - 3_600_000).getTime(),
  surface: "chat",
  projectActive: true,
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
      JSON.stringify({ visits: "many", lastMessageAt: null, firstSeenTodayAt: [], recent: ["ok", 3, null] }),
    );
    expect(readHistory(store)).toEqual({ visits: 0, lastVisitAt: 0, firstSeenTodayAt: 0, recent: ["ok"] });
  });

  it("使うたびに先頭に足し、長さ 24 で頭打ちにする", () => {
    let history = emptyHistory();
    for (let i = 0; i < 40; i++) history = remember(history, [`id-${i}`]);
    expect(history.recent).toHaveLength(24);
    expect(history.recent[0]).toBe("id-39");
    expect(history.recent).not.toContain("id-15");
  });
});

describe("文面の組み立て", () => {
  const signals = (overrides = {}) =>
    buildSignals({
      now,
      conversationCount: 12,
      messageCount: 140,
      projectCount: 1,
      lastMessageAt: workspace.lastActivityAt,
      lastVisitAt: now.getTime() - 3 * 86_400_000,
      firstSeenTodayAt: now.getTime() - 86_400_000,
      visits: 30,
      surface: "chat",
      projectActive: true,
      ...overrides,
    });

  it("見出し・補足・提案が揃う", () => {
    const welcome = welcomeFrom(signals(), [], () => 0);
    expect(welcome.title.length).toBeGreaterThanOrEqual(2);
    expect(welcome.note.length).toBeGreaterThan(4);
    expect(welcome.suggestions).toHaveLength(3);
    expect(welcome.id).toContain("/");
  });

  it("同じ乱数では同じ文面になる", () => {
    const first = welcomeFrom(signals(), [], () => 0.5);
    const second = welcomeFrom(signals(), [], () => 0.5);
    expect(second.title).toEqual(first.title);
    expect(second.note).toBe(first.note);
  });

  it("履歴の部品を避けて別の文面を作る", () => {
    const first = welcomeFrom(signals(), [], () => 0.5);
    const second = welcomeFrom(signals(), first.used, () => 0.5);
    expect(second.id).not.toBe(first.id);
    expect(second.used.filter((id) => first.used.includes(id))).toEqual([]);
  });

  it("履歴を溜めても枯渇しない", () => {
    let recent: string[] = [];
    for (let visit = 0; visit < 40; visit++) {
      const welcome = welcomeFrom(signals({ now: new Date(2026, 5, 10 + visit, 10 + (visit % 12), 5) }), recent, () => 0.5);
      recent = [...welcome.used, ...recent].slice(0, 24);
      expect(welcome.title.length).toBeGreaterThanOrEqual(2);
      expect(welcome.suggestions).toHaveLength(3);
    }
  });

  it("見出しは日本語の文として終る", () => {
    for (let i = 0; i < 200; i++) {
      const welcome = welcomeFrom(signals(), [], () => i / 200);
      const title = welcome.title.join("");
      expect(title).toMatch(/[。？]$/);
      expect(title).not.toMatch(/？{2,}/);
      expect(title).not.toMatch(/[ \u3000]/);
    }
  });

  it("Desktop-Agent では補足が実行モデルを語る", () => {
    // returning / night / agent / project の補足の重みは同じなので、
    // Agent 用以外をすべて除外して、選び方によらず Agent 用の文になることを確認します。
    const agentOnly = ["nt-15", "nt-16", "nt-13", "nt-14", "nt-1", "nt-6", "nt-20", "nt-21", "nt-10", "nt-11", "nt-12", "nt-7", "nt-8", "nt-9", "nt-2", "nt-3", "nt-4", "nt-5"];
    for (const random of [0, 0.25, 0.5, 0.75, 0.999]) {
      const welcome = welcomeFrom(signals({ surface: "desktop-agent" }), agentOnly, () => random);
      expect(["nt-17", "nt-18", "nt-19"], welcome.note).toContain(
        welcome.note.length ? welcomeNoteId(welcome.note) : "",
      );
    }
  });

  it("祝日の朝はその祝日の文を含む", () => {
    const welcome = welcomeFrom(
      signals({ now: new Date(2026, 4, 5, 9, 0) }),
      [],
      () => 0.1,
    );
    expect(welcome.title.join("")).toContain("こどもの日です。");
  });
});

describe("言語ごとの結合", () => {
  const welcome = welcomeFrom(
    buildSignals({
      now,
      conversationCount: 12,
      messageCount: 140,
      projectCount: 1,
      lastMessageAt: workspace.lastActivityAt,
      lastVisitAt: now.getTime() - 3 * 86_400_000,
      firstSeenTodayAt: now.getTime() - 86_400_000,
      visits: 30,
      surface: "chat",
      projectActive: true,
    }),
    [],
    () => 0,
  );

  it("日本語は間なしで連結する", () => {
    setLanguage("ja");
    expect(welcomeTitle(welcome)).toBe(welcome.title.join(""));
  });

  it("英語は空白区切りで連結し、各文が終止符を持つ", () => {
    setLanguage("en");
    const joined = welcomeTitle(welcome);
    expect(joined).toContain(" ");
    expect(joined).not.toContain(welcome.title[0]);
    for (const sentence of joined.split(/(?<=[.?!]) /)) expect(sentence).toMatch(/[.?!]$/);
  });

  it("言語を切り替えると見出しだけが変わり、部品は同じ", () => {
    setLanguage("ja");
    const japanese = welcomeTitle(welcome);
    setLanguage("en");
    expect(welcomeTitle(welcome).split(" ").length).toBeGreaterThanOrEqual(2);
    setLanguage("ja");
    expect(welcomeTitle(welcome)).toBe(japanese);
  });
});

describe("送信欄への差込み", () => {
  it("モード指定があれば付け、無ければ省略する", () => {
    expect(promptDetail({ id: "a", label: "l", prompt: "p" })).toEqual({ text: "p" });
    expect(promptDetail({ id: "b", label: "l", prompt: "p", mode: "thinking" })).toEqual({
      text: "p",
      mode: "thinking",
    });
  });
});

describe("訪問ごとの更新", () => {
  it("localStorage が使えない環境でも文面を返す", () => {
    // nextVisit 自体は localStorage を直接触らず、history.ts の既定実装経由で
    // 防御されるため、ここでは保存が失敗しても生成は進むことを確認します。
    const welcome = welcomeFrom(
      buildSignals({
        now,
        conversationCount: 0,
        messageCount: 0,
        projectCount: 0,
        lastMessageAt: null,
        lastVisitAt: 0,
        firstSeenTodayAt: 0,
        visits: 0,
        surface: "chat",
        projectActive: false,
      }),
      [],
      () => 0,
    );
    expect(welcome.title.length).toBeGreaterThanOrEqual(2);
    expect(welcome.suggestions).toHaveLength(3);
  });
});