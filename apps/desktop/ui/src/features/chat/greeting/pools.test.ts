import { describe, expect, it } from "vitest";
import { greetingEn } from "./translations";
import { suggestionLabelId, suggestionPromptId } from "./translate";
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
    if (
      value &&
      typeof value === "object" &&
      "id" in value &&
      "form" in value
    ) {
      const fragment = value as { id: unknown; text: unknown; form: unknown };
      out.push({
        id: String(fragment.id),
        text: String(fragment.text),
        form: String(fragment.form),
      });
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
    for (const fragment of allFragments())
      expect(fragment.text, fragment.id).toMatch(allowed);
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
      const expected = fragment.text.endsWith("？")
        ? "question"
        : "declarative";
      expect(fragment.form, fragment.id).toBe(expected);
    }
  });

  it("すべての部品と提案に英訳がある", () => {
    for (const fragment of allFragments()) {
      expect(greetingEn[fragment.id], fragment.id).toBeTruthy();
    }
    const suggestions = suggestionsFor(signals, [], 30, () => 0);
    expect(suggestions.length).toBeGreaterThanOrEqual(3);
    for (const suggestion of suggestions) {
      expect(
        greetingEn[suggestionLabelId(suggestion.id)],
        suggestion.id,
      ).toBeTruthy();
      expect(
        greetingEn[suggestionPromptId(suggestion.id)],
        suggestion.id,
      ).toBeTruthy();
    }
  });

  it("他言語の文字が混入していない", () => {
    // 生成の乱れで、簡体字（例: 电脑）やハングルが本文へ混ざった事例が
    // ありました。漢字・かなの許容テストは漢字なら通してしまうため、
    // 簡体字専用字とハングルを個別に弾きます。
    const hangul = /[\uac00-\ud7a3\u1100-\u11ff\u3130-\u318f]/;
    const simplifiedOnly =
      /[对电脑们这说让认动时进过还图头议请谢见报务备处场长门问间关单应该种类级总结语读听么边发开东车马鸟鱼龙风飞书买卖爱乐丽专义习乡云亚产亲从仓仪价众优传伤华协卫历压厌双变叹团园围圣坏块坚坛担拟拥择挂换据摄摆无显术杀权极构树样检楼欢欧岁毕汇汉汤沟泽洁测浏浑浓涂涌润涨渐渔渗湾湿满滚滨滩灭灯灵灾炉炼烂烛烟烦烧热爷牵牺犹狮狱猎环现琼畅疗疮疯瘫]/u;
    for (const fragment of allFragments()) {
      expect(fragment.text, fragment.id).not.toMatch(hangul);
      expect(fragment.text, fragment.id).not.toMatch(simplifiedOnly);
    }
  });
});
