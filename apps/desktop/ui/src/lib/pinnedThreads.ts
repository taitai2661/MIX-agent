import { getJSON, setJSON } from "./storage";

const PINNED_THREADS_KEY = "mix:pinned-threads";

function isStringArray(val: unknown): val is string[] {
  return Array.isArray(val) && val.every((item) => typeof item === "string");
}

export function loadPinnedThreadIds(store?: Storage | null): string[] {
  return getJSON<string[]>(PINNED_THREADS_KEY, [], isStringArray, store);
}

export function savePinnedThreadIds(ids: string[], store?: Storage | null): void {
  setJSON(PINNED_THREADS_KEY, ids, store);
}

export function togglePinnedThreadId(id: string, store?: Storage | null): string[] {
  const current = loadPinnedThreadIds(store);
  const exists = current.includes(id);
  const next = exists ? current.filter((item) => item !== id) : [id, ...current];
  savePinnedThreadIds(next, store);
  return next;
}
