/** Reliable plaintext copy for the Tauri WebView + browsers.
 *
 * `navigator.clipboard.writeText` alone is not enough here: inside the Tauri
 * WebView (custom protocol, focus/permission quirks on WKWebView/WebView2)
 * it can be missing or reject with `NotAllowedError`. The old `CopyButton`
 * swallowed that rejection and still showed "Copied", so a failed copy looked
 * successful — that is the "copy bugs out" report.
 *
 * `copyText` tries the async Clipboard API first, then falls back to a hidden
 * textarea + `document.execCommand("copy")`, which works in insecure contexts
 * and older WebViews. It resolves `true` only when text actually reached the
 * clipboard, so callers can render success vs failure honestly.
 */
export async function copyText(value: string): Promise<boolean> {
  if (!value) return false;

  try {
    if (
      typeof navigator !== "undefined" &&
      navigator.clipboard &&
      typeof navigator.clipboard.writeText === "function"
    ) {
      await navigator.clipboard.writeText(value);
      return true;
    }
  } catch {
    // Fall through to the execCommand fallback below.
  }

  return legacyCopy(value);
}

function legacyCopy(value: string): boolean {
  try {
    if (typeof document === "undefined") return false;
    const active = document.activeElement as HTMLElement | null;

    const textarea = document.createElement("textarea");
    textarea.value = value;
    // Keep it out of view without `display: none` (which breaks selection).
    textarea.style.position = "fixed";
    textarea.style.top = "-9999px";
    textarea.style.left = "-9999px";
    textarea.style.opacity = "0";
    textarea.setAttribute("readonly", "");
    textarea.setAttribute("aria-hidden", "true");
    document.body.appendChild(textarea);

    textarea.focus({ preventScroll: true });
    textarea.select();
    // iOS Safari needs an explicit range.
    textarea.setSelectionRange(0, textarea.value.length);

    const ok = document.execCommand("copy");
    textarea.remove();
    if (active && typeof active.focus === "function") {
      try {
        active.focus({ preventScroll: true } as FocusOptions);
      } catch {
        active.focus();
      }
    }
    return ok;
  } catch {
    return false;
  }
}
