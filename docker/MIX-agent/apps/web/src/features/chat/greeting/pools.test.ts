import { describe, expect, it } from "vitest";
import { en } from "@/app/i18n";
import * as pools from "./pools";
import { suggestionsFor } from "./suggestions";
import { buildSignals } from "./signals";

function fragments(value: unknown, out: { id: string; text: string }[] = []) {
  if (Array.isArray(value)) {
    value.forEach((item) => fragments(item, out));
    return out;
  }
  if (value && typeof value === "object" && "id" in value && "text" in value)
    out.push({ id: String((value as { id: unknown }).id), text: String((value as { text: unknown }).text) });
  else if (value && typeof value === "object")
    Object.values(value as Record<string, unknown>).forEach((item) => fragments(item, out));
  return out;
}

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
    if (value && typeof value === "object")
      Object.values(value as Record<string, unknown>).forEach(walk);
  };
  walk(pools);
  return out;
}

const now = new Date(2026, 0, 1, 9, 0);
const signals = buildSignals({
  now,
  conversationCount: 12,
  pinnedCount: 2,
  projectCount: 1,
  agentCount: 1,
  lastMessageAt: now.getTime() - 3_600_000,
  lastVisitAt: 0,
  firstSeenTodayAt: 0,
  visits: 0,
});

describe("挨拶のパーツ", () => {
  it("すべて日本語の文で終わり、文体タグを持つ", () => {
    const all = fragments(pools);
    expect(all.length).toBeGreaterThan(200);
    for (const fragment of all) {
      expect(fragment.id, fragment.text).toMatch(/^[a-z]{2,3}-\d+$/);
      expect(fragment.text.length, fragment.id).toBeGreaterThan(4);
    }
  });

it("日本語の文として妥当な文字だけを使う", () => {
    // 句読点・記号・漢字・かな・カタカナ・拉丁文字のみ。
    // 小書きの仮名と長音符は Unicode 上 Common 属するため個別に許します。
    // 生成の乱れで混入した他言語や記号を検出します。
    const allowed =
      /^[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}ぁぃぅぇぉっゃゅょゔァィゥェォッャュョヵヶーA-Za-z0-9・、。？「」（）]+$/u;
    for (const fragment of fragments(pools)) expect(fragment.text, fragment.id).toMatch(allowed);
  });

  it("ID と本文が重複しない", () => {
    const all = fragments(pools);
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
    const suggestions = suggestionsFor(signals, [], 12, () => 0);
    expect(suggestions.length).toBeGreaterThanOrEqual(3);
    for (const suggestion of suggestions) {
      expect(Object.keys(en), suggestion.id).toContain(suggestion.label);
      expect(Object.keys(en), suggestion.id).toContain(suggestion.prompt);
    }
  });
});