/// Theme handling.
///
/// The UI never keys off `prefers-color-scheme` directly. `system` is resolved
/// to a concrete `light` / `dark` here and written to `data-theme`, so there is
/// exactly one source of truth for "is the app dark" and the Tailwind `dark:`
/// variant reduces to a single attribute selector.

import { getString, setString } from "./storage";

export type Theme = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

const STORAGE_KEY = "mix-agent-theme";
const QUERY = "(prefers-color-scheme: dark)";

export function normalizeTheme(value: unknown): Theme {
  return value === "light" || value === "dark" || value === "system"
    ? value
    : "system";
}

function prefersDark(): boolean {
  return (
    typeof window !== "undefined" && window.matchMedia?.(QUERY).matches === true
  );
}

export function resolveTheme(theme: Theme): ResolvedTheme {
  if (theme === "system") return prefersDark() ? "dark" : "light";
  return theme;
}

let watching: MediaQueryList | null = null;

/**
 * Mirrors the resolved theme onto the native window.
 *
 * `data-theme` and `color-scheme` only reach content WebKit renders itself.
 * Native chrome — the `<select>` popup this app uses for every dropdown, the
 * context menus, the traffic-light bar — keeps the *system* appearance. On a
 * Mac running dark mode with the app set to light, the preset picker therefore
 * opened as a dark, translucent panel with the white form showing through it,
 * unreadable next to the rest of the pane. Pushing the theme onto the window
 * makes those native menus match the UI again.
 *
 * Best-effort: non-Tauri (plain `vite dev`) has no window to talk to, and a
 * build whose ACL omits `core:window:allow-set-theme` simply keeps following
 * the system appearance.
 *
 * `system` is passed as `null` (follow the OS) rather than the currently
 * resolved value: pinning the window would also pin the WebView's
 * `prefers-color-scheme`, so the media-query listener would stop firing and a
 * "system" app would never notice the OS switching to the other theme.
 */
function syncNativeTheme(theme: Theme): void {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    return;
  }
  const target = theme === "system" ? null : theme;
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(target))
    .catch(() => {});
}

/** Writes the concrete theme to the document and the native window. */
function setResolvedTheme(theme: Theme): void {
  document.documentElement.dataset.theme = resolveTheme(theme);
  syncNativeTheme(theme);
}

/**
 * Writes `data-theme` to a concrete value. Safe to call repeatedly: the OS
 * listener is installed at most once, so the settings pane can call this on
 * every toggle without stacking subscriptions.
 */
export function applyTheme(theme: Theme): void {
  if (typeof document === "undefined") return;
  setResolvedTheme(theme);

  if (typeof window === "undefined" || !window.matchMedia) return;
  const mql = window.matchMedia(QUERY);
  if (watching === mql) return;

  watching?.removeEventListener("change", onSystemChange);
  watching = mql;
  mql.addEventListener("change", onSystemChange);
}

function onSystemChange(): void {
  // Only react while the user has actually deferred to the OS.
  if (storedTheme() !== "system") return;
  setResolvedTheme("system");
}

export function storedTheme(): Theme {
  return normalizeTheme(getString(STORAGE_KEY));
}

export function storeTheme(theme: Theme): void {
  setString(STORAGE_KEY, theme);
  applyTheme(theme);
}
