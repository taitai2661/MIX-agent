/// Platform-aware shortcut labels.
///
/// `App` publishes the runtime platform on
/// `document.documentElement.dataset.platform` (`"macos"` | `"other"`),
/// preferring Tauri's `plugin-os` over UA sniffing so a macOS WebView that
/// reports a Windows-ish UA still gets the traffic lights.
///
/// The keyboard handlers accept both `metaKey` and `ctrlKey`; this module keeps
/// the *displayed* caps in step (`⌘` / `⇧` on macOS, `Ctrl` / `Shift`
/// elsewhere) instead of hard-coding the macOS glyphs into every hint.

const MAC_UA = /Macintosh|Mac OS X/i;

export function isMacPlatform(): boolean {
  if (typeof document !== "undefined") {
    const platform = document.documentElement.dataset.platform;
    if (platform) return platform === "macos";
  }
  if (typeof navigator === "undefined") return true;
  return MAC_UA.test(navigator.userAgent);
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
