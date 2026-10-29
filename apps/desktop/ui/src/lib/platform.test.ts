import { afterEach, describe, expect, it } from "vitest";
import {
  chord,
  detectPlatformFromUa,
  isLinuxPlatform,
  isMacPlatform,
  isWindowsPlatform,
  modKey,
  runtimePlatform,
  shiftKey,
} from "./platform";

describe("platform shortcut labels", () => {
  const originalPlatform = document.documentElement.dataset.platform;

  afterEach(() => {
    if (originalPlatform !== undefined) {
      document.documentElement.dataset.platform = originalPlatform;
    } else {
      delete document.documentElement.dataset.platform;
    }
  });

  it("uses the macOS glyphs when the runtime platform is macos", () => {
    document.documentElement.dataset.platform = "macos";
    expect(runtimePlatform()).toBe("macos");
    expect(isMacPlatform()).toBe(true);
    expect(isWindowsPlatform()).toBe(false);
    expect(isLinuxPlatform()).toBe(false);
    expect(modKey()).toBe("⌘");
    expect(shiftKey()).toBe("⇧");
    expect(chord("Mod", "Shift", "E")).toBe("⌘⇧E");
    expect(chord("Mod", "Enter")).toBe("⌘Enter");
  });

  it("recognises Windows and Linux separately", () => {
    document.documentElement.dataset.platform = "windows";
    expect(runtimePlatform()).toBe("windows");
    expect(isWindowsPlatform()).toBe(true);
    expect(isMacPlatform()).toBe(false);

    document.documentElement.dataset.platform = "linux";
    expect(runtimePlatform()).toBe("linux");
    expect(isLinuxPlatform()).toBe(true);
    expect(isMacPlatform()).toBe(false);
  });

  it("uses Ctrl/Shift labels on non-mac platforms", () => {
    document.documentElement.dataset.platform = "other";
    expect(runtimePlatform()).toBe("other");
    expect(isMacPlatform()).toBe(false);
    expect(modKey()).toBe("Ctrl");
    expect(shiftKey()).toBe("Shift");
    expect(chord("Mod", "Shift", "E")).toBe("Ctrl+Shift+E");
    expect(chord("Mod", "Enter")).toBe("Ctrl+Enter");
  });

  it("detects the platform from a user agent", () => {
    expect(detectPlatformFromUa("Mozilla/5.0 (Windows NT 10.0)")).toBe(
      "windows",
    );
    expect(detectPlatformFromUa("Mozilla/5.0 (X11; Linux x86_64)")).toBe(
      "linux",
    );
    expect(
      detectPlatformFromUa("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"),
    ).toBe("macos");
    expect(detectPlatformFromUa("Mozilla/5.0 (iPhone)")).toBe("other");
  });

  it("leaves literal keys untouched inside a chord", () => {
    document.documentElement.dataset.platform = "other";
    expect(chord("Mod", ",")).toBe("Ctrl+,");
    document.documentElement.dataset.platform = "macos";
    expect(chord("Mod", ",")).toBe("⌘,");
  });
});
