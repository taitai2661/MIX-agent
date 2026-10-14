import { getString, setString } from "@/lib/storage";

const key = "mix-agent-welcome";
const MAX_RECENT = 24;

export interface WelcomeHistory {
  visits: number;
  lastVisitAt: number;
  firstSeenTodayAt: number;
  recent: string[];
}

export function emptyHistory(): WelcomeHistory {
  return { visits: 0, lastVisitAt: 0, firstSeenTodayAt: 0, recent: [] };
}

export function readHistory(store?: Storage | null): WelcomeHistory {
  const raw = getString(key, null, store);
  if (!raw) return emptyHistory();
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    // Corrupt payload (hand-edited, truncated write): reset to empty loudly
    // instead of silently dropping history every launch.
    console.warn("[welcome] ignoring corrupt history; resetting to empty");
    return emptyHistory();
  }
  if (!parsed || typeof parsed !== "object") {
    console.warn("[welcome] ignoring malformed history; resetting to empty");
    return emptyHistory();
  }
  const value = parsed as Record<string, unknown>;
  const number = (input: unknown) =>
    typeof input === "number" && Number.isFinite(input) ? input : 0;
  return {
    visits: Math.max(0, Math.trunc(number(value.visits))),
    lastVisitAt: number(value.lastVisitAt),
    firstSeenTodayAt: number(value.firstSeenTodayAt),
    recent: Array.isArray(value.recent)
      ? value.recent.filter((item): item is string => typeof item === "string").slice(0, MAX_RECENT)
      : [],
  };
}

export function writeHistory(history: WelcomeHistory, store?: Storage | null): void {
  try {
    setString(key, JSON.stringify(history), store);
  } catch (error) {
    // QuotaExceededError and friends: the greeting still renders, but say so
    // loudly instead of silently dropping history every launch.
    console.warn("[welcome] could not persist history:", error);
  }
}

/** 今回表示した部品の ID を先頭に足し、長さ一定に収めます。 */
export function remember(history: WelcomeHistory, used: readonly string[]): WelcomeHistory {
  return {
    ...history,
    recent: [...used, ...history.recent].slice(0, MAX_RECENT),
  };
}