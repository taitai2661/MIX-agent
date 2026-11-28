/// Command-palette frecency.
///
/// The palette has always opened on a fixed order: primary actions first, then
/// recent threads. That is a good default, but it never learns that one person
/// opens "診断" or "ブラウザ" ten times a day. This module keeps a tiny,
/// best-effort usage map in localStorage and moves the commands the user
/// actually reaches for to the top of the empty-query list.
///
/// Only ids and timestamps are stored — never labels, query text or content.

import { getJSON, setJSON } from "./storage";

export const PALETTE_USAGE_KEY = "mix-agent-palette-usage";
/** Keep the map bounded; old entries are dropped on every write. */
const MAX_ENTRIES = 40;
/** How many learned rows may outrank the fixed default order. */
export const MAX_PROMOTED = 6;

export interface PaletteUsageEntry {
  count: number;
  /** Epoch milliseconds of the last activation. */
  last: number;
}

export type PaletteUsage = Record<string, PaletteUsageEntry>;

function isEntry(value: unknown): value is PaletteUsageEntry {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.count === "number" &&
    Number.isFinite(record.count) &&
    record.count > 0 &&
    typeof record.last === "number" &&
    Number.isFinite(record.last)
  );
}

function isUsage(value: unknown): value is PaletteUsage {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return Object.values(record).every(isEntry);
}

export function loadPaletteUsage(store?: Storage | null): PaletteUsage {
  return getJSON(PALETTE_USAGE_KEY, {}, isUsage, store);
}

/** Record one activation and persist the trimmed map. */
export function recordPaletteUse(
  id: string,
  now = Date.now(),
  store?: Storage | null,
): PaletteUsage {
  const usage = loadPaletteUsage(store);
  const previous = usage[id];
  const next: PaletteUsage = {
    ...usage,
    [id]: { count: (previous?.count ?? 0) + 1, last: now },
  };
  const trimmed: PaletteUsage = {};
  for (const [key, entry] of Object.entries(next)
    .sort(([, a], [, b]) => b.last - a.last)
    .slice(0, MAX_ENTRIES)) {
    trimmed[key] = entry;
  }
  setJSON(PALETTE_USAGE_KEY, trimmed, store);
  return trimmed;
}

/**
 * Move recently-used items ahead of the default order.
 *
 * Threads and messages are included on purpose: "continue where I left off"
 * should reflect the thread the user actually reopened, not only commands.
 * The number of promoted rows is capped so the primary actions never slide
 * out of the first screenful.
 */
export function rankByFrecency<T extends { id: string }>(
  items: T[],
  usage: PaletteUsage,
  maxPromoted = MAX_PROMOTED,
): T[] {
  const used: { item: T; entry: PaletteUsageEntry; index: number }[] = [];
  const rest: T[] = [];
  items.forEach((item, index) => {
    const entry = usage[item.id];
    if (entry) used.push({ item, entry, index });
    else rest.push(item);
  });
  if (used.length === 0) return items;
  used.sort((a, b) => b.entry.last - a.entry.last || a.index - b.index);
  const promoted = used.slice(0, maxPromoted).map(({ item }) => item);
  const promotedIds = new Set(promoted.map((item) => item.id));
  return [...promoted, ...items.filter((item) => !promotedIds.has(item.id))];
}
