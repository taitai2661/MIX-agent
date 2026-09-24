import { afterEach, describe, expect, it } from "vitest";
import { browserLanguage, getLanguage, setLanguage, t } from "./i18n";

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
});
