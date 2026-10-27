import { afterEach, describe, expect, it } from "vitest";
import { chord, isMacPlatform, modKey, shiftKey } from "./platform";

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
    expect(isMacPlatform()).toBe(true);
    expect(modKey()).toBe("⌘");
    expect(shiftKey()).toBe("⇧");
    expect(chord("Mod", "Shift", "E")).toBe("⌘⇧E");
    expect(chord("Mod", "Enter")).toBe("⌘Enter");
  });

  it("uses Ctrl/Shift labels on non-mac platforms", () => {
    document.documentElement.dataset.platform = "other";
    expect(isMacPlatform()).toBe(false);
    expect(modKey()).toBe("Ctrl");
    expect(shiftKey()).toBe("Shift");
    expect(chord("Mod", "Shift", "E")).toBe("Ctrl+Shift+E");
    expect(chord("Mod", "Enter")).toBe("Ctrl+Enter");
  });

  it("leaves literal keys untouched inside a chord", () => {
    document.documentElement.dataset.platform = "other";
    expect(chord("Mod", ",")).toBe("Ctrl+,");
    document.documentElement.dataset.platform = "macos";
    expect(chord("Mod", ",")).toBe("⌘,");
  });
});
