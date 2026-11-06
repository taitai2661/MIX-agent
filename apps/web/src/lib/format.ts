import { getLanguage } from "@/app/i18n";

export const HISTORY_GROUPS = ["today", "yesterday", "week", "older"] as const;
export type HistoryGroup = (typeof HISTORY_GROUPS)[number];

/** Bucket a conversation by its last activity, using local calendar days. */
export function historyGroup(value: unknown): HistoryGroup {
  const date = new Date(String(value ?? ""));
  if (Number.isNaN(date.getTime())) return "older";
  const now = new Date();
  const startOfToday = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate(),
  ).getTime();
  const stamp = date.getTime();
  if (stamp >= startOfToday) return "today";
  if (stamp >= startOfToday - 86_400_000) return "yesterday";
  if (stamp >= startOfToday - 6 * 86_400_000) return "week";
  return "older";
}

/** Compact timestamp for list rows: time for today, date otherwise. */
export function historyTime(value: unknown): string {
  const date = new Date(String(value ?? ""));
  if (Number.isNaN(date.getTime())) return "";
  const locale = getLanguage() === "en" ? "en" : "ja";
  const sameDay = date.toDateString() === new Date().toDateString();
  return new Intl.DateTimeFormat(
    locale,
    sameDay
      ? { hour: "2-digit", minute: "2-digit" }
      : { month: "numeric", day: "numeric" },
  ).format(date);
}
