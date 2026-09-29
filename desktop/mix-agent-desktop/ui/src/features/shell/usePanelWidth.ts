import { useState } from "react";

export function readPanelWidth(
  key: string,
  fallback: number,
  min: number,
  max: number,
): number {
  try {
    const stored = Number(localStorage.getItem(key));
    if (Number.isFinite(stored) && stored >= min && stored <= max) return stored;
  } catch {
    // Storage is best-effort; use the layout default when unavailable.
  }
  return fallback;
}

export function usePanelWidth(key: string, fallback: number, min: number, max: number) {
  const [width, setWidth] = useState(() => readPanelWidth(key, fallback, min, max));
  const commitWidth = (next: number) => {
    try {
      localStorage.setItem(key, String(next));
    } catch {
      // Persisting a panel width is best-effort.
    }
  };
  return { width, setWidth, commitWidth };
}
