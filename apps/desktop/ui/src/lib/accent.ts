/// Accent colour handling.
///
/// The desktop palette keeps one accent hue for focus, selection and primary
/// actions (see `styles/tokens.css`). Those tokens are the single source of
/// truth; this module only swaps the concrete brand colour the user picked,
/// and re-derives the translucent tints from it so `bg-brand/15` and
/// `ring-brand/20` keep working with any hue.
///
/// Persisted in `localStorage`, like the other display preferences (zoom,
/// reduced motion): the backend settings row is for shared configuration,
/// while an accent is a per-installation visual choice.

import { getString, setString } from "./storage";

export type Accent =
  | "blue"
  | "green"
  | "indigo"
  | "violet"
  | "rose"
  | "amber"
  | "teal";

export type AccentTheme = "light" | "dark";

type Palette = {
  brand: string;
  /** Slightly stronger on hover — used by primary buttons. */
  hover: string;
  /** The high-contrast member of the ramp. */
  strong: string;
};

export type AccentOption = {
  id: Accent;
  label: string;
  palette: Record<AccentTheme, Palette>;
};

/**
 * Blue is first and is the default: it is the hue the desktop shell already
 * shipped with, so existing installs see no change until they pick another
 * swatch.
 */
export const ACCENTS: AccentOption[] = [
  {
    id: "blue",
    label: "青",
    palette: {
      light: { brand: "#3b6ef0", hover: "#2f5fdc", strong: "#2749a8" },
      dark: { brand: "#6f96ff", hover: "#8aa9ff", strong: "#9db8ff" },
    },
  },
  {
    id: "green",
    label: "緑",
    palette: {
      light: { brand: "#167c5b", hover: "#126b4e", strong: "#0e6248" },
      dark: { brand: "#54c79a", hover: "#6ad4a8", strong: "#72d8ad" },
    },
  },
  {
    id: "indigo",
    label: "藍",
    palette: {
      light: { brand: "#4f46e5", hover: "#453cd4", strong: "#4338ca" },
      dark: { brand: "#8f8bf5", hover: "#a0a0f7", strong: "#a9a6f8" },
    },
  },
  {
    id: "violet",
    label: "紫",
    palette: {
      light: { brand: "#7c3aed", hover: "#7130dd", strong: "#6d28d9" },
      dark: { brand: "#b58cf5", hover: "#c19ff7", strong: "#c9a9f8" },
    },
  },
  {
    id: "rose",
    label: "ピンク",
    palette: {
      light: { brand: "#db2777", hover: "#cc246f", strong: "#be185d" },
      dark: { brand: "#f472b6", hover: "#f686c2", strong: "#f79ac6" },
    },
  },
  {
    id: "amber",
    label: "オレンジ",
    palette: {
      light: { brand: "#b45309", hover: "#a34a08", strong: "#92400e" },
      dark: { brand: "#f0a94a", hover: "#f3b563", strong: "#f4bd72" },
    },
  },
  {
    id: "teal",
    label: "ティール",
    palette: {
      light: { brand: "#0f766e", hover: "#0d6a63", strong: "#0b5f59" },
      dark: { brand: "#4cc6bc", hover: "#5dcfc6", strong: "#72d6cd" },
    },
  },
];

export const DEFAULT_ACCENT: Accent = "blue";
export const ACCENT_STORAGE_KEY = "mix-agent-accent";

export function normalizeAccent(value: unknown): Accent {
  return ACCENTS.some((option) => option.id === value)
    ? (value as Accent)
    : DEFAULT_ACCENT;
}

/** Restores the persisted accent; unknown or stale values fall back safely. */
export function storedAccent(store?: Storage | null): Accent {
  return normalizeAccent(getString(ACCENT_STORAGE_KEY, null, store));
}

/** Persists and immediately paints a newly selected accent. */
export function storeAccent(accent: Accent, store?: Storage | null): void {
  setString(ACCENT_STORAGE_KEY, accent, store);
  applyAccent(accent, currentTheme());
}

function currentTheme(): AccentTheme {
  if (typeof document === "undefined") return "light";
  return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

/** `#rrggbb` → `rgba(r, g, b, alpha)` for translucent tints. */
function withAlpha(hex: string, alpha: number): string {
  const value = hex.replace("#", "");
  const r = Number.parseInt(value.slice(0, 2), 16);
  const g = Number.parseInt(value.slice(2, 4), 16);
  const b = Number.parseInt(value.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/**
 * Writes the accent ramp onto the document.
 *
 * Inline custom properties override the `@theme` defaults in tokens.css, so
 * every utility built on `--color-brand` follows the pick automatically.
 * `soft` and `focus` are derived from the chosen brand rather than listed per
 * palette: they are tints of the same hue by definition, and keeping them in
 * step is what makes arbitrary opacity modifiers (`bg-brand/10`) work.
 */
export function applyAccent(accent: Accent, theme: AccentTheme): void {
  if (typeof document === "undefined") return;
  const option = ACCENTS.find((entry) => entry.id === accent) ?? ACCENTS[0];
  const palette = option.palette[theme];
  const style = document.documentElement.style;
  style.setProperty("--color-brand", palette.brand);
  style.setProperty("--color-brand-hover", palette.hover);
  style.setProperty("--color-brand-strong", palette.strong);
  style.setProperty(
    "--color-brand-soft",
    withAlpha(palette.brand, theme === "dark" ? 0.16 : 0.1),
  );
  style.setProperty(
    "--color-focus",
    withAlpha(palette.brand, theme === "dark" ? 0.45 : 0.35),
  );
  style.setProperty("--color-accent", palette.brand);
}
