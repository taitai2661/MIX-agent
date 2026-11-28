import { act, render, renderHook } from "@testing-library/react";
import { createElement, memo } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  browserLanguage,
  getLanguage,
  setLanguage,
  subscribeLanguage,
  t,
  tf,
  useLanguage,
  useT,
} from "./i18n";

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
    expect(browserLanguage(["de-DE"])).toBe("en");
    expect(browserLanguage([])).toBe("en");
  });

  it("notifies subscribers when the language changes", () => {
    const listener = vi.fn();
    const unsubscribe = subscribeLanguage(listener);
    setLanguage("en");
    expect(listener).toHaveBeenCalledTimes(1);
    expect(getLanguage()).toBe("en");
    unsubscribe();
    setLanguage("ja");
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("re-renders useLanguage consumers after setLanguage", () => {
    setLanguage("ja");
    const { result } = renderHook(() => useLanguage());
    expect(result.current).toBe("ja");
    act(() => setLanguage("en"));
    expect(result.current).toBe("en");
    act(() => setLanguage("ja"));
  });

  it("updates memoized t() consumers after setLanguage", () => {
    const MemoLeaf = memo(function MemoLeaf() {
      const tt = useT();
      return createElement("span", null, tt("送信"));
    });
    setLanguage("ja");
    const { container } = render(createElement(MemoLeaf));
    expect(container.textContent).toBe("送信");
    act(() => setLanguage("en"));
    expect(container.textContent).toBe("Send");
    act(() => setLanguage("ja"));
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
