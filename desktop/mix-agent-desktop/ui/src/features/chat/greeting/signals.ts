import { calendarDate, eventOfDay, holidayOf, solarTermOf } from "./calendar";
import type {
  Activity,
  DayKind,
  Gap,
  PartOfDay,
  Season,
  SeasonPhase,
  Signals,
  Surface,
  Workload,
} from "./types";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

export function partOfDay(date: Date): PartOfDay {
  const hour = date.getHours();
  if (hour >= 5 && hour < 7) return "earlyMorning";
  if (hour >= 7 && hour < 9) return "morning";
  if (hour >= 9 && hour < 12) return "lateMorning";
  if (hour >= 12 && hour < 14) return "midday";
  if (hour >= 14 && hour < 17) return "afternoon";
  if (hour >= 17 && hour < 19) return "dusk";
  if (hour >= 19 && hour < 22) return "evening";
  return "lateNight";
}

export function dayKind(date: Date): DayKind {
  const day = date.getDay();
  if (day === 0) return "sunday";
  if (day === 6) return "weekend";
  if (day === 1) return "monday";
  if (day === 5) return "friday";
  return "weekday";
}

export function seasonOf(date: Date): { season: Season; seasonPhase: SeasonPhase } {
  const month = date.getMonth() + 1;
  const season: Season =
    month >= 3 && month <= 5
      ? "spring"
      : month >= 6 && month <= 8
        ? "summer"
        : month >= 9 && month <= 11
          ? "autumn"
          : "winter";
  const start = season === "spring" ? 3 : season === "summer" ? 6 : season === "autumn" ? 9 : 12;
  const offset = ((month - start) + 12) % 12;
  return { season, seasonPhase: offset === 0 ? "early" : offset === 1 ? "mid" : "late" };
}

export function dayOfYear(date: Date): number {
  const { year, month, day } = calendarDate(date);
  return Math.round(
    (Date.UTC(year, month - 1, day) - Date.UTC(year, 0, 1)) / 86_400_000,
  ) + 1;
}

export function workloadOf(conversationCount: number): Workload {
  if (conversationCount <= 0) return "empty";
  if (conversationCount <= 4) return "light";
  if (conversationCount <= 19) return "steady";
  return "heavy";
}

/** 前回の最終活動からの経過を、粗い粒度で判定します。 */
export function activityOf(lastMessageAt: number | null, now: Date): Activity {
  if (lastMessageAt === null) return "none";
  const elapsed = now.getTime() - lastMessageAt;
  if (elapsed < 45 * MINUTE) return "justNow";
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (lastMessageAt >= today) return "today";
  if (lastMessageAt >= today - DAY) return "yesterday";
  if (lastMessageAt >= today - 7 * DAY) return "thisWeek";
  return "earlier";
}

export function gapOf(now: Date, lastVisitAt: number): Gap {
  if (!lastVisitAt) return "first";
  const elapsed = now.getTime() - lastVisitAt;
  if (elapsed < 5 * MINUTE) return "reload";
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (lastVisitAt >= today) return "sameDay";
  if (lastVisitAt >= today - DAY) return "overnight";
  if (elapsed < 7 * DAY) return "fewDays";
  if (elapsed < 30 * DAY) return "longAway";
  return "farAway";
}

export function isFirstVisitOfDay(now: Date, firstSeenTodayAt: number): boolean {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  return firstSeenTodayAt < today;
}

export function buildSignals(input: {
  now: Date;
  conversationCount: number;
  messageCount: number;
  projectCount: number;
  lastMessageAt: number | null;
  lastVisitAt: number;
  firstSeenTodayAt: number;
  visits: number;
  surface: Surface;
  projectActive: boolean;
}): Signals {
  const { now } = input;
  return {
    now,
    partOfDay: partOfDay(now),
    dayKind: dayKind(now),
    ...seasonOf(now),
    solarTerm: solarTermOf(now)?.name ?? null,
    holiday: holidayOf(now)?.name ?? null,
    event: eventOfDay(now)?.name ?? null,
    yearProgress: yearProgress(now),
    conversationCount: input.conversationCount,
    messageCount: input.messageCount,
    projectCount: input.projectCount,
    surface: input.surface,
    projectActive: input.projectActive,
    lastMessageAt: input.lastMessageAt,
    activity: activityOf(input.lastMessageAt, now),
    gap: gapOf(now, input.lastVisitAt),
    visits: input.visits,
    firstToday: isFirstVisitOfDay(now, input.firstSeenTodayAt),
  };
}

function yearProgress(now: Date): Signals["yearProgress"] {
  const day = dayOfYear(now);
  if (day <= 31) return "start";
  if (day >= 335) return "end";
  return null;
}