import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { setTheme } = vi.hoisted(() => ({ setTheme: vi.fn() }));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme }),
}));

import { applyTheme } from "./theme";

type TauriWindow = Window & { __TAURI_INTERNALS__?: unknown };

const nativeWindow = window as TauriWindow;

function mockSystemPrefersDark(dark: boolean): void {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (query: string): MediaQueryList =>
      ({
        matches: query.includes("dark") && dark,
        media: query,
        onchange: null,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
      }) as unknown as MediaQueryList,
  });
}

describe("applyTheme", () => {
  beforeEach(() => {
    localStorage.clear();
    setTheme.mockReset();
    setTheme.mockResolvedValue(undefined);
    mockSystemPrefersDark(false);
    nativeWindow.__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete nativeWindow.__TAURI_INTERNALS__;
    document.documentElement.removeAttribute("data-theme");
  });

  it("paints the document and pins the native window to an explicit theme", async () => {
    applyTheme("dark");

    expect(document.documentElement.dataset.theme).toBe("dark");
    // Native chrome (select popups, context menus) follows the window
    // appearance, not `color-scheme`; without this it stays dark on a
    // dark-mode Mac even when the app is set to light, and vice versa.
    await vi.waitFor(() => expect(setTheme).toHaveBeenCalledWith("dark"));
  });

  it("lets the OS own the appearance while the theme is `system`", async () => {
    applyTheme("system");

    expect(document.documentElement.dataset.theme).toBe("light");
    // Pinning the window here would also pin `prefers-color-scheme`, and the
    // system listener in `applyTheme` would never see the OS switch.
    await vi.waitFor(() => expect(setTheme).toHaveBeenCalledWith(null));
  });

  it("does not talk to a window outside Tauri", async () => {
    delete nativeWindow.__TAURI_INTERNALS__;
    applyTheme("light");

    await Promise.resolve();
    expect(setTheme).not.toHaveBeenCalled();
  });
});
