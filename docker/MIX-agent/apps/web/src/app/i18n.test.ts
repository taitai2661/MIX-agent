import { afterEach, describe, expect, it } from "vitest";
import { browserLanguage, getLanguage, setLanguage, t } from "./i18n";
import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import { Auth } from "@/features/auth/Auth";

afterEach(() => setLanguage("ja"));

describe("display language", () => {
  it("uses the first supported browser language", () => {
    expect(browserLanguage(["fr-FR", "en-US", "ja-JP"])).toBe("en");
    expect(browserLanguage(["ja-JP", "en-US"])).toBe("ja");
    expect(browserLanguage(["fr-FR"])).toBe("ja");
  });

  it("switches the translated text immediately", () => {
    setLanguage("en");
    expect(getLanguage()).toBe("en");
    expect(t("初期セットアップ")).toBe("Initial setup");
    setLanguage("ja");
    expect(t("初期セットアップ")).toBe("初期セットアップ");
  });

  it("translates greeting suggestion prompts too", () => {
    // 提案ラベルだけでなく、送信欄へ差し込む本文も翻訳キーであることを固定します。
    setLanguage("en");
    expect(t("MIX agent の使い方を、実際の場面で教えてください。")).toBe(
      "Show me how MIX agent works, using a realistic situation.",
    );
  });

  it("renders administrator creation in English", () => {
    setLanguage("en");
    const html = renderToStaticMarkup(createElement(Auth, { setup: true, onDone: () => {} }));
    expect(html).toContain("Welcome to MIX agent.");
    expect(html).toContain("Create workspace");
    expect(html).toContain('value="en" selected=""');
    expect(html).not.toContain("ようこそ");
  });
});
