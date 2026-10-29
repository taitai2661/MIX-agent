/// Platform-aware shortcut labels and feature hints.
///
/// `App` publishes the runtime platform on
/// `document.documentElement.dataset.platform` (`"macos"` | `"windows"` |
/// `"linux"` | `"other"`), preferring Tauri's `plugin-os` over UA sniffing so a
/// macOS WebView that reports a Windows-ish UA still gets the traffic lights.
///
/// The keyboard handlers accept both `metaKey` and `ctrlKey`; this module keeps
/// the *displayed* caps in step (`⌘` / `⇧` on macOS, `Ctrl` / `Shift`
/// elsewhere) instead of hard-coding the macOS glyphs into every hint.

export type RuntimePlatform = "macos" | "windows" | "linux" | "other";

const MAC_UA = /Macintosh|Mac OS X/i;
const WINDOWS_UA = /Windows/i;
const LINUX_UA = /Linux|X11/i;

/** Best-effort platform from a user agent, for the first paint. */
export function detectPlatformFromUa(userAgent: string): RuntimePlatform {
  if (WINDOWS_UA.test(userAgent)) return "windows";
  if (LINUX_UA.test(userAgent)) return "linux";
  if (MAC_UA.test(userAgent)) return "macos";
  return "other";
}

/** The OS the UI is currently running on. */
export function runtimePlatform(): RuntimePlatform {
  if (typeof document !== "undefined") {
    const platform = document.documentElement.dataset.platform;
    if (
      platform === "macos" ||
      platform === "windows" ||
      platform === "linux" ||
      platform === "other"
    ) {
      return platform;
    }
  }
  if (typeof navigator === "undefined") return "macos";
  return detectPlatformFromUa(navigator.userAgent);
}

export function isMacPlatform(): boolean {
  return runtimePlatform() === "macos";
}

export function isWindowsPlatform(): boolean {
  return runtimePlatform() === "windows";
}

export function isLinuxPlatform(): boolean {
  return runtimePlatform() === "linux";
}

/** Primary modifier cap: `⌘` on macOS, `Ctrl` elsewhere. */
export function modKey(): string {
  return isMacPlatform() ? "⌘" : "Ctrl";
}

/** Shift cap: `⇧` on macOS, `Shift` elsewhere. */
export function shiftKey(): string {
  return isMacPlatform() ? "⇧" : "Shift";
}

/**
 * A compact chord for titles and hints: `⌘⇧E` on macOS, `Ctrl+Shift+E`
 * elsewhere. Pass `"Mod"` / `"Shift"` as tokens so the platform is decided at
 * render time (the dataset is set in a layout effect).
 */
export function chord(...keys: readonly string[]): string {
  const parts = keys.map((key) =>
    key === "Mod" ? modKey() : key === "Shift" ? shiftKey() : key,
  );
  return isMacPlatform() ? parts.join("") : parts.join("+");
}
