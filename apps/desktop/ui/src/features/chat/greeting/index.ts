import { getLanguage } from "@/lib/i18n";
import { compose } from "./compose";
import { readHistory, remember, writeHistory } from "./history";
import { buildSignals } from "./signals";
import { suggestionsFor } from "./suggestions";
import { greetingText, suggestionPromptId } from "./translate";
import type { Signals, Suggestion, Surface, Welcome } from "./types";

export {
  activityOf,
  buildSignals,
  dayKind,
  dayOfYear,
  gapOf,
  partOfDay,
  seasonOf,
  workloadOf,
} from "./signals";
export {
  calendarDate,
  dayNumber,
  eventOfDay,
  holidayOf,
  solarTermOf,
  upcomingSolarTerm,
} from "./calendar";
export {
  emptyHistory,
  readHistory,
  remember,
  writeHistory,
  type WelcomeHistory,
} from "./history";
export { compatible, pickFragment } from "./compose";
export { suggestionsFor } from "./suggestions";
export {
  greetingText,
  suggestionLabelId,
  suggestionPromptId,
} from "./translate";
export type {
  Activity,
  DayKind,
  Form,
  Fragment,
  Gap,
  PartOfDay,
  Season,
  SeasonPhase,
  Signals,
  Suggestion,
  Surface,
  Tone,
  Welcome,
  Workload,
} from "./types";

export interface WorkspaceSnapshot {
  conversationCount: number;
  messageCount: number;
  projectCount: number;
  /** 最も新しいスレッドの最終更新時刻（epoch ms）。無ければ null。 */
  lastActivityAt: number | null;
  surface: Surface;
  projectActive: boolean;
}

export function welcomeFrom(
  signals: Signals,
  recent: readonly string[] = [],
  random: () => number = Math.random,
): Welcome {
  const composed = compose(signals, recent, random);
  // 見出しで使った部品は提案側でも再利用しないよう、同じ除外集合を渡します。
  const suggestions = suggestionsFor(
    signals,
    [...recent, ...composed.used],
    3,
    random,
  );
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
  const midnight = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate(),
  ).getTime();
  const signals = buildSignals({
    now,
    conversationCount: workspace.conversationCount,
    messageCount: workspace.messageCount,
    projectCount: workspace.projectCount,
    lastMessageAt: workspace.lastActivityAt,
    lastVisitAt: history.lastVisitAt,
    firstSeenTodayAt: history.firstSeenTodayAt,
    visits: history.visits,
    surface: workspace.surface,
    projectActive: workspace.projectActive,
  });
  const welcome = welcomeFrom(signals, history.recent, random);
  const updated = remember(
    {
      visits: history.visits + 1,
      lastVisitAt: now.getTime(),
      firstSeenTodayAt:
        history.firstSeenTodayAt < midnight
          ? now.getTime()
          : history.firstSeenTodayAt,
      recent: history.recent,
    },
    welcome.used,
  );
  writeHistory(updated);
  return { welcome, history: updated };
}

/** 提案を送信欄に差し込むため、`usePromptInsert` の引数へ変換します。 */
export function promptDetail(suggestion: Suggestion): {
  text: string;
  mode?: Suggestion["mode"];
} {
  // 提案の文言も表示言語へ翻訳してから差し込みます。ラベルだけ翻訳して
  // 本文が日本語のまま送信欄へ入ると、英語 UI で言語が混ざります。
  const text = greetingText(
    suggestionPromptId(suggestion.id),
    suggestion.prompt,
  );
  return suggestion.mode ? { text, mode: suggestion.mode } : { text };
}
