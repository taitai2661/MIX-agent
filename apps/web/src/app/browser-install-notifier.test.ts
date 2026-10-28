import { afterEach, describe, expect, it } from "vitest";
import { setLanguage } from "./i18n";
import { browserInstallNotice } from "./browser-install-notifier";

afterEach(() => setLanguage("ja"));

describe("Chromium install popups", () => {
  it("announces a newly started install", () => {
    expect(browserInstallNotice("not_installed", "installing")).toMatchObject({
      type: "info",
      title: "Chromiumをインストールしています",
    });
    expect(browserInstallNotice("ready", "installing")).toMatchObject({
      type: "info",
    });
  });

  it("announces completion and failure only after an install", () => {
    expect(browserInstallNotice("installing", "ready")).toMatchObject({
      type: "success",
      title: "Chromiumのインストールが完了しました",
    });
    expect(browserInstallNotice("installing", "failed")).toMatchObject({
      type: "error",
      title: "Chromiumのインストールに失敗しました",
    });
    expect(browserInstallNotice("not_installed", "failed")).toBeNull();
  });

  it("stays quiet on the first read and on unrelated changes", () => {
    expect(browserInstallNotice(null, "ready")).toBeNull();
    expect(browserInstallNotice(null, "installing")).toMatchObject({
      type: "info",
    });
    expect(browserInstallNotice("ready", "ready")).toBeNull();
  });

  it("translates the popup for English users", () => {
    setLanguage("en");
    expect(browserInstallNotice("installing", "ready")).toMatchObject({
      type: "success",
      title: "Chromium installation finished",
      message: "The Browser tool is ready to use.",
    });
  });
});
