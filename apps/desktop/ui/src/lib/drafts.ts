/// Per-conversation composer drafts.
///
/// Drafts are convenient but they are also plain text on disk: a pasted token
/// or private note must not live in localStorage forever. Records therefore
/// carry a timestamp and expire, and the Settings pane can clear all of them in
/// one action (see `countComposerDrafts` / `clearComposerDrafts`).
///
/// Older builds stored the bare text. That shape is still read as a legacy
/// draft; the next write upgrades it to the timestamped record.

import { getString, listKeys, removeKey, removeKeys, setJSON } from "./storage";

const PREFIX = "mix:composer-draft:";
export const COMPOSER_DRAFT_TTL_MS = 30 * 24 * 60 * 60 * 1000;

interface StoredDraft {
  text: string;
  savedAt: number;
}

function keyFor(slot: string | null): string {
  return `${PREFIX}${slot ?? "none"}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function parse(value: string): StoredDraft | null {
  try {
    const parsed: unknown = JSON.parse(value);
    if (
      isRecord(parsed) &&
      typeof parsed.text === "string" &&
      typeof parsed.savedAt === "number"
    ) {
      return { text: parsed.text, savedAt: parsed.savedAt };
    }
  } catch {
    // Legacy plain-text draft.
  }
  return null;
}

/** Load a draft, dropping records older than the TTL. */
export function loadComposerDraft(
  slot: string | null,
  now = Date.now(),
): string {
  const key = keyFor(slot);
  const raw = getString(key);
  if (!raw) return "";
  const record = parse(raw);
  if (!record) return raw;
  if (now - record.savedAt > COMPOSER_DRAFT_TTL_MS) {
    removeKey(key);
    return "";
  }
  return record.text;
}

export function saveComposerDraft(
  slot: string | null,
  text: string,
  now = Date.now(),
): void {
  setJSON(keyFor(slot), { text, savedAt: now } satisfies StoredDraft);
}

export function clearComposerDraft(slot: string | null): void {
  removeKey(keyFor(slot));
}

export function countComposerDrafts(store?: Storage | null): number {
  return listKeys(PREFIX, store).length;
}

/** Drop expired records on boot so a stale draft cannot linger forever just
 *  because its thread was never reopened. */
export function pruneComposerDrafts(now = Date.now()): number {
  let removed = 0;
  for (const key of listKeys(PREFIX)) {
    const raw = getString(key);
    if (!raw) continue;
    const record = parse(raw);
    if (record && now - record.savedAt > COMPOSER_DRAFT_TTL_MS) {
      removeKey(key);
      removed += 1;
    }
  }
  return removed;
}

/** Remove every draft, including expired ones. Returns the number removed. */
export function clearComposerDrafts(store?: Storage | null): number {
  return removeKeys(PREFIX, store);
}
