import { describe, expect, it } from "vitest";
import { dictionary } from "@/lib/i18n";
import * as pools from "./pools";
import { suggestionsFor } from "./suggestions";
import { buildSignals } from "./signals";

function allFragments() {
  const out: { id: string; text: string; form: string }[] = [];
  const walk = (value: unknown) => {
    if (Array.isArray(value)) {
      value.forEach(walk);
      return;
    }
    if (value && typeof value === "object" && "id" in value && "form" in value) {
      const fragment = value as { id: unknown; text: unknown; form: unknown };
      out.push({ id: String(fragment.id), text: String(fragment.text), form: String(fragment.form) });
      return;
    }
    if (value && typeof value === "object") Object.values(value as Record<string, unknown>).forEach(walk);
  };
  walk(pools);
  return out;
}

const now = new Date(2026, 0, 1, 9, 0);
const signals = buildSignals({
  now,
  conversationCount: 12,
  messageCount: 40,
  projectCount: 1,
  lastMessageAt: now.getTime() - 3_600_000,
  lastVisitAt: now.getTime() - 3 * 86_400_000,
  firstSeenTodayAt: now.getTime() - 86_400_000,
  visits: 30,
  surface: "chat",
  projectActive: true,
});

describe("挨拶のパーツ", () => {
  it("部品が十分な数だけ登録されている", () => {
    expect(allFragments().length).toBeGreaterThan(240);
  });

  it("ID の形式がそろっている", () => {
    for (const fragment of allFragments()) {
      expect(fragment.id, fragment.text).toMatch(/^[a-z]{2,3}-\d+$/);
      expect(fragment.text.length, fragment.id).toBeGreaterThan(4);
    }
  });

  it("日本語の文として妥当な文字だけを使う", () => {
    // 句読点・記号・漢字・かな・カタカナ・拉丁文字のみ。
    // 小書きの仮名と長音符は Unicode 上 Common に属するため個別に許します。
    const allowed =
      /^[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}ぁぃぅぇぉっゃゅょゔァィゥェォッャュョヵヶーA-Za-z0-9 ・、。？「」（）]+$/u;
    for (const fragment of allFragments()) expect(fragment.text, fragment.id).toMatch(allowed);
  });

  it("ID と本文が重複しない", () => {
    const all = allFragments();
    expect(new Set(all.map((fragment) => fragment.id)).size).toBe(all.length);
    expect(new Set(all.map((fragment) => fragment.text)).size).toBe(all.length);
  });

  it("文形のタグが文末記号と一致する", () => {
    // 合成の制約は form で判定するため、実文と食い違うと
    // 「？」「？」が連続しても弾けないことがあります。
    for (const fragment of allFragments()) {
      const expected = fragment.text.endsWith("？") ? "question" : "declarative";
      expect(fragment.form, fragment.id).toBe(expected);
    }
  });

  it("提案の文言もすべて英語訳を持つ", () => {
    const keys = new Set(Object.keys(dictionary));
    const suggestions = suggestionsFor(signals, [], 30, () => 0);
    expect(suggestions.length).toBeGreaterThanOrEqual(3);
    for (const suggestion of suggestions) {
      expect(keys, suggestion.id).toContain(suggestion.label);
      expect(keys, suggestion.id).toContain(suggestion.prompt);
    }
  });
});