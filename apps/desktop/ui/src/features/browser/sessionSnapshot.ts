import { getJSON, removeKey, setJSON } from "@/lib/storage";

/** One page we can recreate after the native WebView was closed. */
export interface BrowserSessionTab {
  url: string;
  title: string;
}

/** Enough to rebuild the tab strip on the next launch. */
export interface BrowserSessionSnapshot {
  tabs: BrowserSessionTab[];
  /** Index into `tabs` of the tab that was active. */
  activeIndex: number;
  savedAt: number;
}

const PREFIX = "mix-agent-browser-session:";
/** A week-old tab set is usually stale (and may point at expired sessions). */
const MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;

export function browserSessionKey(workspaceId: string): string {
  return `${PREFIX}${workspaceId}`;
}

function isSnapshot(value: unknown): value is BrowserSessionSnapshot {
  if (!value || typeof value !== "object") return false;
  const snapshot = value as Record<string, unknown>;
  if (!Array.isArray(snapshot.tabs)) return false;
  if (typeof snapshot.savedAt !== "number") return false;
  if (typeof snapshot.activeIndex !== "number") return false;
  return snapshot.tabs.every((tab) => {
    if (!tab || typeof tab !== "object") return false;
    const entry = tab as Record<string, unknown>;
    return typeof entry.url === "string" && typeof entry.title === "string";
  });
}

export function readBrowserSession(
  workspaceId: string,
  now: number = Date.now(),
): BrowserSessionSnapshot | null {
  const snapshot = getJSON<BrowserSessionSnapshot | null>(
    browserSessionKey(workspaceId),
    null,
    (value): value is BrowserSessionSnapshot => isSnapshot(value),
  );
  if (!snapshot) return null;
  if (now - snapshot.savedAt > MAX_AGE_MS) {
    removeKey(browserSessionKey(workspaceId));
    return null;
  }
  return snapshot;
}

export function saveBrowserSession(
  workspaceId: string,
  snapshot: BrowserSessionSnapshot,
): void {
  setJSON(browserSessionKey(workspaceId), snapshot);
}

export function clearBrowserSession(workspaceId: string): void {
  removeKey(browserSessionKey(workspaceId));
}
