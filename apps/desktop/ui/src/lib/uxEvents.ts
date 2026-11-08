/** Local, privacy-preserving UX event counters.
 *
 * Phase 0 of `docs/desktop/ux-improvement-plan.md` needs a baseline for the
 * journeys users actually take (first message, approvals, settings, search).
 * Sending that data anywhere would contradict the app's local-first promise,
 * so this module keeps a small aggregate in WebStorage instead:
 *
 * - one counter per event name, with first/last timestamps
 * - the *last* detail object for that event (surface/mode/location only)
 * - never prompts, messages, paths or any other user content
 *
 * The aggregate is surfaced in Diagnostics and can be cleared from there.
 * Writes are best-effort through `lib/storage`, so private windows and tests
 * that stub WebStorage simply drop them.
 */

import { getJSON, setJSON, removeKey } from "./storage";

export type UxEventName =
  | "message_sent"
  | "run_completed"
  | "new_thread"
  | "surface_switched"
  | "approval_shown"
  | "approval_decided"
  | "settings_opened"
  | "settings_saved"
  | "settings_save_failed"
  | "search_opened"
  | "panel_opened"
  | "error_shown";

export interface UxEventRecord {
  count: number;
  first_at: string;
  last_at: string;
  /** Last non-identifying context (surface, mode, location, …). */
  detail?: Record<string, string | number | boolean | null>;
}

interface UxEventStore {
  version: 1;
  events: Record<string, UxEventRecord>;
}

export const UX_EVENTS_KEY = "mix-agent-ux-events";

const MAX_EVENTS = 40;

function isStore(value: unknown): value is UxEventStore {
  if (!value || typeof value !== "object") return false;
  const store = value as Partial<UxEventStore>;
  return (
    store.version === 1 &&
    !!store.events &&
    typeof store.events === "object" &&
    !Array.isArray(store.events)
  );
}

function readStore(): UxEventStore {
  return getJSON<UxEventStore>(
    UX_EVENTS_KEY,
    { version: 1, events: {} },
    isStore,
  );
}

/** Record one event. Safe to call from render paths, but prefer handlers so
 *  StrictMode's double render cannot inflate a counter. */
export function recordUxEvent(
  name: UxEventName,
  detail?: UxEventRecord["detail"],
): void {
  const store = readStore();
  const now = new Date().toISOString();
  const previous = store.events[name];
  const record: UxEventRecord = previous
    ? {
        count: previous.count + 1,
        first_at: previous.first_at,
        last_at: now,
        ...(detail
          ? { detail }
          : previous.detail
            ? { detail: previous.detail }
            : {}),
      }
    : {
        count: 1,
        first_at: now,
        last_at: now,
        ...(detail ? { detail } : {}),
      };

  const events: Record<string, UxEventRecord> = {
    ...store.events,
    [name]: record,
  };

  // Keep the aggregate bounded: drop the least recently seen event when a
  // future event name is added without pruning the old ones.
  const names = Object.keys(events);
  if (names.length > MAX_EVENTS) {
    names
      .sort((a, b) => events[a]!.last_at.localeCompare(events[b]!.last_at))
      .slice(0, names.length - MAX_EVENTS)
      .forEach((old) => delete events[old]);
  }

  setJSON(UX_EVENTS_KEY, { version: 1, events });
}

export interface UxEventSummary extends UxEventRecord {
  name: string;
}

/** All recorded events, newest activity first. */
export function readUxEvents(): UxEventSummary[] {
  const store = readStore();
  return Object.entries(store.events)
    .map(([name, record]) => ({ name, ...record }))
    .sort((a, b) => b.last_at.localeCompare(a.last_at));
}

export function clearUxEvents(): void {
  removeKey(UX_EVENTS_KEY);
}
