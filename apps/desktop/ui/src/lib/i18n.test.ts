import { afterEach, describe, expect, it } from "vitest";
import { browserLanguage, setLanguage, t, tf } from "./i18n";

describe("i18n", () => {
  afterEach(() => setLanguage("ja"));

  it("defaults to Japanese (keys pass through)", () => {
    setLanguage("ja");
    expect(t("新規スレッド")).toBe("新規スレッド");
    expect(t("未知の文字列")).toBe("未知の文字列");
  });

  it("maps known keys to English", () => {
    setLanguage("en");
    expect(t("新規スレッド")).toBe("New thread");
    expect(t("今日")).toBe("Today");
  });

  it("falls back to the key when a translation is missing", () => {
    setLanguage("en");
    expect(t("未翻訳のキー")).toBe("未翻訳のキー");
  });

  it("interpolates placeholders", () => {
    setLanguage("ja");
    expect(
      tf("ステップ上限 {steps} / ツール上限 {calls} に到達しました。", {
        steps: 300,
        calls: 750,
      }),
    ).toBe("ステップ上限 300 / ツール上限 750 に到達しました。");
    setLanguage("en");
    expect(
      tf("ステップ上限 {steps} / ツール上限 {calls} に到達しました。", {
        steps: 1,
        calls: 2,
      }),
    ).toBe("Reached the step limit (1) / tool limit (2).");
  });

  it("detects the browser language", () => {
    expect(browserLanguage(["ja-JP", "en"])).toBe("ja");
    expect(browserLanguage(["en-US"])).toBe("en");
    expect(browserLanguage(["de-DE"])).toBe("ja");
    expect(browserLanguage([])).toBe("ja");
  });

  it("translates relativeTime labels", async () => {
    const { relativeTime } = await import("./format");
    const now = new Date(2026, 0, 15, 12, 0, 0);
    const ago = (minutes: number) =>
      new Date(now.getTime() - minutes * 60_000).toISOString();
    setLanguage("en");
    expect(relativeTime(ago(0), now)).toBe("Just now");
    expect(relativeTime(ago(30), now)).toBe("30 min ago");
    expect(relativeTime(ago(5 * 60), now)).toBe("5h ago");
    expect(relativeTime(new Date(2026, 0, 12, 12).toISOString(), now)).toBe(
      "3d ago",
    );
    setLanguage("ja");
    expect(relativeTime(ago(30), now)).toBe("30分前");
  });
});
