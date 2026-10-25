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

/** localStorage はプライベートモードなどで例外になるため、常に防御して読み書きします。 */
function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

export function readHistory(store: Storage | null = storage()): WelcomeHistory {
  if (!store) return emptyHistory();
  let raw: string | null = null;
  try {
    raw = store.getItem(key);
  } catch {
    return emptyHistory();
  }
  if (!raw) return emptyHistory();
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return emptyHistory();
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
  } catch {
    return emptyHistory();
  }
}

export function writeHistory(history: WelcomeHistory, store: Storage | null = storage()): void {
  if (!store) return;
  try {
    store.setItem(key, JSON.stringify(history));
  } catch {
    // 保存できなくても挨拶の生成自体は続ける。
  }
}

/** 今回表示した部品の ID を先頭に足し、長さ一定に収めます。 */
export function remember(history: WelcomeHistory, used: readonly string[]): WelcomeHistory {
  return {
    ...history,
    recent: [...used, ...history.recent].slice(0, MAX_RECENT),
  };
}