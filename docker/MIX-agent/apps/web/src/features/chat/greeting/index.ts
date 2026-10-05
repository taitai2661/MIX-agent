import { getLanguage } from "@/app/i18n";
import { compose } from "./compose";
import { readHistory, remember, writeHistory } from "./history";
import { buildSignals } from "./signals";
import { suggestionsFor } from "./suggestions";
import { greetingText } from "./translate";
import type { Signals, Welcome } from "./types";

export { activityOf, buildSignals, dayKind, dayOfYear, gapOf, partOfDay, seasonOf, workloadOf } from "./signals";
export {
  calendarDate,
  dayNumber,
  eventOfDay,
  holidayOf,
  solarTermOf,
  upcomingSolarTerm,
} from "./calendar";
export { emptyHistory, readHistory, writeHistory, remember, type WelcomeHistory } from "./history";
export { compatible, pickFragment } from "./compose";
export { suggestionsFor } from "./suggestions";
export { greetingText, suggestionLabelId, suggestionPromptId } from "./translate";
export type {
  Activity,
  DayKind,
  Fragment,
  Gap,
  PartOfDay,
  Season,
  SeasonPhase,
  Signals,
  Suggestion,
  Tone,
  Welcome,
  Workload,
} from "./types";

export interface WorkspaceSnapshot {
  conversationCount: number;
  pinnedCount: number;
  projectCount: number;
  agentCount: number;
  lastMessageAt: number | null;
}

/**
 * 見出し・補足・提案をまとめて 1 文面作ります。
 * 直近で使った部品は next.recent から除外するため、同じ挨拶の連続を避けられます。
 */
export function pickWelcome(
  workspace: WorkspaceSnapshot,
  next: { recent: readonly string[] },
  random: () => number = Math.random,
  now: Date = new Date(),
): Welcome {
  const signals = buildSignals({
    now,
    conversationCount: workspace.conversationCount,
    pinnedCount: workspace.pinnedCount,
    projectCount: workspace.projectCount,
    agentCount: workspace.agentCount,
    lastMessageAt: workspace.lastMessageAt,
    lastVisitAt: 0,
    firstSeenTodayAt: 0,
    visits: 0,
  });
  return welcomeFrom(signals, next.recent, random);
}

export function welcomeFrom(
  signals: Signals,
  recent: readonly string[] = [],
  random: () => number = Math.random,
): Welcome {
  const composed = compose(signals, recent, random);
  // 見出しで使った部品は提案側でも再利用しないよう、同じ除外集合を渡します。
  const suggestions = suggestionsFor(signals, [...recent, ...composed.used], 3, random);
  return {
    id: composed.used.join("/"),
    title: composed.title,
    titleIds: composed.titleIds,
    note: composed.note.text,
    noteId: composed.note.id,
    suggestions,
    used: [...composed.used, ...suggestions.map((suggestion) => suggestion.id)],
  };
}

/**
 * 文単位に分解した日本語の文を、表示言語で 1 文へまとめます。
 * 日本語は間なしで連結し、英語は各文が終止符を持つため空白で連結します。
 * 翻訳は日本語原文ではなく部品 ID で引くため、原文を直しても英語は壊れません。
 */
export function welcomeTitle(welcome: Welcome): string {
  if (getLanguage() === "ja") return welcome.title.join("");
  return welcome.title
    .map((text, index) => greetingText(welcome.titleIds[index] ?? "", text))
    .join(" ");
}

/** 補足の 1 文を表示言語で返します。 */
export function welcomeNote(welcome: Welcome): string {
  return greetingText(welcome.noteId, welcome.note);
}

export interface VisitResult {
  welcome: Welcome;
  history: ReturnType<typeof remember>;
}

/** ブラウザに保存された訪問履歴を反映して、次回以降の挨拶へ引き継ぎます。 */
export function nextVisit(
  workspace: WorkspaceSnapshot,
  random: () => number = Math.random,
  now: Date = new Date(),
): VisitResult {
  const history = readHistory();
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const signals = buildSignals({
    now,
    conversationCount: workspace.conversationCount,
    pinnedCount: workspace.pinnedCount,
    projectCount: workspace.projectCount,
    agentCount: workspace.agentCount,
    lastMessageAt: workspace.lastMessageAt,
    lastVisitAt: history.lastVisitAt,
    firstSeenTodayAt: history.firstSeenTodayAt,
    visits: history.visits,
  });
  const welcome = welcomeFrom(signals, history.recent, random);
  const updated = remember(
    {
      visits: history.visits + 1,
      lastVisitAt: now.getTime(),
      firstSeenTodayAt: history.firstSeenTodayAt < midnight ? now.getTime() : history.firstSeenTodayAt,
      recent: history.recent,
    },
    welcome.used,
  );
  writeHistory(updated);
  return { welcome, history: updated };
}
