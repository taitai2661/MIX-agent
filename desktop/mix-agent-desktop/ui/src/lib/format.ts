/// Small display formatters shared across panes.

/** "たった今" / "3分前" / "昨日" / "1月2日" style relative label. */
export function relativeTime(iso: string, now: Date = new Date()): string {
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "";
  const diffMs = now.getTime() - then.getTime();
  const minutes = Math.floor(diffMs / 60_000);
  if (diffMs < 60_000) return "たった今";
  if (minutes < 60) return `${minutes}分前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}時間前`;
  const days = Math.floor(hours / 24);
  if (days === 1) return "昨日";
  if (days < 7) return `${days}日前`;
  return then.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** Sidebar grouping bucket for a timestamp. */
export type ThreadGroup = "today" | "yesterday" | "week" | "older";

export function threadGroup(iso: string, now: Date = new Date()): ThreadGroup {
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "older";
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const time = then.getTime();
  if (time >= startOfToday) return "today";
  if (time >= startOfToday - 86_400_000) return "yesterday";
  if (time >= startOfToday - 7 * 86_400_000) return "week";
  return "older";
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return seconds ? `${minutes}m ${seconds}s` : `${minutes}m`;
}

/** Short id label for runs/tools ("a1b2c3d4"). */
export function shortId(id: string, len = 8): string {
  return id.slice(0, len);
}
