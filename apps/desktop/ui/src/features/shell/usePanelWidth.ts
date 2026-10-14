import { useState } from "react";
import { getString, setString } from "@/lib/storage";

export function readPanelWidth(
  key: string,
  fallback: number,
  min: number,
  max: number,
): number {
  const raw = getString(key);
  if (raw === null) return fallback;
  const stored = Number(raw);
  if (Number.isFinite(stored) && stored >= min && stored <= max) return stored;
  return fallback;
}

export function usePanelWidth(key: string, fallback: number, min: number, max: number) {
  const [width, setWidth] = useState(() => readPanelWidth(key, fallback, min, max));
  const commitWidth = (next: number) => {
    // Clamp into [min, max]: drag handlers can overshoot by a frame, and an
    // out-of-range value would fail `readPanelWidth` on the next mount and
    // silently revert to the fallback. Non-finite values are dropped.
    if (!Number.isFinite(next)) return;
    const clamped = Math.min(max, Math.max(min, next));
    setWidth(clamped);
    setString(key, String(clamped));
  };
  return { width, setWidth, commitWidth };
}
