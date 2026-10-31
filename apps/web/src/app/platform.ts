/**
 * Platform-aware labels for the web UI.
 *
 * The browser build runs on every OS, so shortcut hints must not hard-code the
 * macOS glyph. Desktop has a richer helper (Tauri tells it the OS); the web app
 * only needs the primary modifier label and can fall back to the UA.
 */
export function isMacPlatform(): boolean {
  if (typeof navigator === "undefined") return false;
  const ua = navigator.userAgent;
  if (/Macintosh|Mac OS X|iPhone|iPad|iPod/i.test(ua)) return true;
  // `platform` is deprecated but remains the most reliable signal in older
  // Safari/Firefox builds where the UA is frozen.
  return /Mac/i.test(navigator.platform ?? "");
}

/** Primary modifier cap: `⌘` on macOS/iOS, `Ctrl` elsewhere. */
export function modKey(): string {
  return isMacPlatform() ? "⌘" : "Ctrl";
}
