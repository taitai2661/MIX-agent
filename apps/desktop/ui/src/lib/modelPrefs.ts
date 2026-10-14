/// Per-user model preferences kept in `localStorage`.
///
/// The backend knows which models exist, but not which ones *this* user reaches
/// for first. Favourites, recents and the last model used on each surface are
/// local ergonomics: pinning the two or three models you actually use turns a
/// fifty-row list into a two-click choice, and remembering the last pick per
/// surface stops the composer from resetting to the first model every launch.
///
/// Everything here is best-effort. `localStorage` can throw (private windows,
/// disabled storage), so every read falls back to an empty result and every
/// write is swallowed — a preference is never worth failing a chat over.

import { create } from "zustand";
import type { Surface } from "./ui";
import { getJSON, setJSON } from "./storage";

const FAVORITES_KEY = "mix-agent-model-favorites";
const RECENTS_KEY = "mix-agent-model-recents";
const LAST_KEY = "mix-agent-last-model";

/** How many recents to keep. Long enough to cover a working session, short
 *  enough that the "Recently used" section never becomes a second full list. */
export const MAX_RECENT_MODELS = 6;

function isStringList(value: unknown): value is string[] {
  return Array.isArray(value)
    && value.every((item): item is string => typeof item === "string");
}

function readList(key: string): string[] {
  return getJSON(key, [], isStringList);
}

function writeList(key: string, value: string[]): void {
  setJSON(key, value);
  notify();
}

/** Preference-change notifier.
///
/// The composer and the settings pane can both change a favourite, and a plain
/// module read would leave the other one stale until it remounted. This used
/// to be a `window` CustomEvent (`mix:model-prefs`); a tiny zustand store
/// carries the same signal without an ad-hoc event bus, so subscribers work
/// in tests and non-DOM environments too. */
const useModelPrefsVersion = create<{ version: number }>(() => ({ version: 0 }));

function notify(): void {
  useModelPrefsVersion.setState((s) => ({ version: s.version + 1 }));
}

export function getFavoriteModels(): string[] {
  return readList(FAVORITES_KEY);
}

export function isFavoriteModel(id: string): boolean {
  return getFavoriteModels().includes(id);
}

/** Pin or unpin a model. Returns the resulting list so the caller can set state
 *  without a second read. */
export function toggleFavoriteModel(id: string): string[] {
  const current = getFavoriteModels();
  const next = current.includes(id)
    ? current.filter((value) => value !== id)
    : [...current, id];
  writeList(FAVORITES_KEY, next);
  return next;
}

export function getRecentModels(): string[] {
  return readList(RECENTS_KEY);
}

/** Move `id` to the front of the recents list. */
export function recordRecentModel(id: string): string[] {
  const current = getRecentModels().filter((value) => value !== id);
  const next = [id, ...current].slice(0, MAX_RECENT_MODELS);
  writeList(RECENTS_KEY, next);
  return next;
}

interface LastBySurface {
  [surface: string]: string;
}

function isLastBySurface(value: unknown): value is LastBySurface {
  if (!value || typeof value !== "object") return false;
  return Object.values(value).every((item) => typeof item === "string");
}

function readLast(): LastBySurface {
  return getJSON(LAST_KEY, {}, isLastBySurface);
}

export function getLastModelForSurface(surface: Surface): string | null {
  return readLast()[surface] ?? null;
}

export function setLastModelForSurface(surface: Surface, id: string): void {
  setJSON(LAST_KEY, { ...readLast(), [surface]: id });
}

/** Subscribe to preference changes. Returns an unsubscribe function. */
export function subscribeModelPrefs(listener: () => void): () => void {
  return useModelPrefsVersion.subscribe((s, prev) => {
    if (s.version !== prev.version) listener();
  });
}
