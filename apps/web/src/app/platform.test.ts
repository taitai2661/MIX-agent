import { afterEach, describe, expect, it, vi } from "vitest";
import { isMacPlatform, modKey } from "./platform";

afterEach(() => vi.unstubAllGlobals());

describe("web platform labels", () => {
  it("uses the command glyph on macOS", () => {
    vi.stubGlobal("navigator", {
      userAgent:
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
      platform: "MacIntel",
    });
    expect(isMacPlatform()).toBe(true);
    expect(modKey()).toBe("⌘");
  });

  it.each([
    ["Windows", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)", "Win32"],
    ["Linux", "Mozilla/5.0 (X11; Linux x86_64)", "Linux x86_64"],
  ])("uses Ctrl on %s", (_name, userAgent, platform) => {
    vi.stubGlobal("navigator", { userAgent, platform });
    expect(isMacPlatform()).toBe(false);
    expect(modKey()).toBe("Ctrl");
  });
});
