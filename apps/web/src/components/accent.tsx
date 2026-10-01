import { useSyncExternalStore } from "react";
import { t } from "@/app/i18n";

export type Accent = "green" | "blue" | "indigo" | "violet" | "rose" | "amber" | "teal";
export type ResolvedTheme = "light" | "dark";

type Palette = Record<ResolvedTheme, { accent: string; strong: string; soft: string }>;

export type AccentOption = {
  id: Accent;
  label: string;
  palette: Palette;
};

export const ACCENTS: AccentOption[] = [
  {
    id: "green",
    label: "緑",
    palette: {
      light: { accent: "#167c5b", strong: "#0e6248", soft: "#e4f3ed" },
      dark: { accent: "#54c79a", strong: "#72d8ad", soft: "#153c2d" },
    },
  },
  {
    id: "blue",
    label: "青",
    palette: {
      light: { accent: "#2563eb", strong: "#1d4ed8", soft: "#e6edfd" },
      dark: { accent: "#6ba3ff", strong: "#8ebcff", soft: "#16305c" },
    },
  },
  {
    id: "indigo",
    label: "藍",
    palette: {
      light: { accent: "#4f46e5", strong: "#4338ca", soft: "#e9e8fd" },
      dark: { accent: "#8f8bf5", strong: "#a9a6f8", soft: "#262450" },
    },
  },
  {
    id: "violet",
    label: "紫",
    palette: {
      light: { accent: "#7c3aed", strong: "#6d28d9", soft: "#f0e9fd" },
      dark: { accent: "#b58cf5", strong: "#c9a9f8", soft: "#35205c" },
    },
  },
  {
    id: "rose",
    label: "ピンク",
    palette: {
      light: { accent: "#db2777", strong: "#be185d", soft: "#fbe7f1" },
      dark: { accent: "#f472b6", strong: "#f79ac6", soft: "#4a1730" },
    },
  },
  {
    id: "amber",
    label: "オレンジ",
    palette: {
      light: { accent: "#b45309", strong: "#92400e", soft: "#f9eddc" },
      dark: { accent: "#f0a94a", strong: "#f4bd72", soft: "#472f14" },
    },
  },
  {
    id: "teal",
    label: "ティール",
    palette: {
      light: { accent: "#0f766e", strong: "#0b5f59", soft: "#e0f3f1" },
      dark: { accent: "#4cc6bc", strong: "#72d6cd", soft: "#103833" },
    },
  },
];

export const DEFAULT_ACCENT: Accent = "green";
const key = "mix-agent-accent";
const listeners = new Set<() => void>();

function isAccent(value: string | null): value is Accent {
  return ACCENTS.some((option) => option.id === value);
}

function initialAccent(): Accent {
  if (typeof localStorage === "undefined") return DEFAULT_ACCENT;
  const stored = localStorage.getItem(key);
  return isAccent(stored) ? stored : DEFAULT_ACCENT;
}

let accent: Accent = initialAccent();

export function getAccent(): Accent {
  return accent;
}

export function setAccent(value: Accent) {
  if (!isAccent(value) || accent === value) return;
  accent = value;
  if (typeof localStorage !== "undefined") localStorage.setItem(key, value);
  applyAccent(resolvedTheme());
  listeners.forEach((listener) => listener());
}

export function useAccent(): Accent {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getAccent,
    getAccent,
  );
}

function resolvedTheme(): ResolvedTheme {
  if (typeof document === "undefined") return "light";
  return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

export function applyAccent(theme: ResolvedTheme) {
  if (typeof document === "undefined") return;
  const option = ACCENTS.find((entry) => entry.id === accent) ?? ACCENTS[0];
  const palette = option.palette[theme];
  const style = document.documentElement.style;
  style.setProperty("--accent", palette.accent);
  style.setProperty("--accent-strong", palette.strong);
  style.setProperty("--accent-soft", palette.soft);
}

export function AccentPicker() {
  const selected = useAccent();
  return (
    <div className="accent-picker" role="group" aria-label={t("アクセントカラーを選択")}>
      {ACCENTS.map((option) => (
        <button
          aria-label={t(option.label)}
          aria-pressed={selected === option.id}
          className={selected === option.id ? "selected" : ""}
          key={option.id}
          onClick={() => setAccent(option.id)}
          style={{ background: option.palette[resolvedTheme()].accent }}
          title={t(option.label)}
          type="button"
        />
      ))}
    </div>
  );
}
