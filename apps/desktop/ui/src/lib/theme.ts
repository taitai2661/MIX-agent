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
  return value === "light" || value === "dark" || value === "system" ? value : "system";
}

function prefersDark(): boolean {
  return typeof window !== "undefined" && window.matchMedia?.(QUERY).matches === true;
}

export function resolveTheme(theme: Theme): ResolvedTheme {
  if (theme === "system") return prefersDark() ? "dark" : "light";
  return theme;
}

let watching: MediaQueryList | null = null;

/**
 * Writes `data-theme` to a concrete value. Safe to call repeatedly: the OS
 * listener is installed at most once, so the settings pane can call this on
 * every toggle without stacking subscriptions.
 */
export function applyTheme(theme: Theme): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = resolveTheme(theme);

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
  document.documentElement.dataset.theme = prefersDark() ? "dark" : "light";
}

export function storedTheme(): Theme {
  return normalizeTheme(getString(STORAGE_KEY));
}

export function storeTheme(theme: Theme): void {
  setString(STORAGE_KEY, theme);
  applyTheme(theme);
}
