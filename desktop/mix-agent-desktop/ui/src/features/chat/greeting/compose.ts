import {
  activityContexts,
  eventContexts,
  firstOpenings,
  fridayContexts,
  gapContexts,
  holidayContexts,
  lateNightContexts,
  middayContexts,
  mondayContexts,
  morningEdgeContexts,
  nightContexts,
  noProjectContexts,
  notes,
  offerClosings,
  openings,
  projectContexts,
  questionClosings,
  returningOpenings,
  resumeClosings,
  seasonContexts,
  sentence,
  solarTermContexts,
  sundayContexts,
  togetherClosings,
  weekdayContexts,
  weekendContexts,
  workloadContexts,
  yearEndContexts,
  yearStartContexts,
} from "./pools";
import type { Fragment, Signals, Tone } from "./types";

const MAX_ATTEMPTS = 40;
const CONTEXT_CHANCE = 0.62;

function weightOf(fragment: Fragment): number {
  return fragment.weight ?? 1;
}

/** 直近に見た部品を除外したうえで、重みに従って 1 つ選びます。 */
export function pickFragment(
  items: readonly Fragment[],
  recent: ReadonlySet<string>,
  random: () => number,
): Fragment | undefined {
  const unseen = items.filter((item) => !recent.has(item.id));
  const pool = unseen.length ? unseen : items;
  if (!pool.length) return undefined;
  const total = pool.reduce((sum, item) => sum + weightOf(item), 0);
  let threshold = random() * total;
  for (const item of pool) {
    threshold -= weightOf(item);
    if (threshold < 0) return item;
  }
  return pool[pool.length - 1];
}

function withWeight(fragments: readonly Fragment[], weight: number): readonly Fragment[] {
  return fragments.map((fragment) => ({ ...fragment, weight }));
}

/** calm はどの文体とも組み合わせるため、常に相性ありとみなします。 */
function sameTone(a: Tone, b: Tone): boolean {
  return a === b || a === "calm" || b === "calm";
}

/** 「？」の連続や文体の混在で不自然にならない組み合わせだけを採用します。 */
export function compatible(
  opening: Fragment,
  context: Fragment | null,
  closing: Fragment,
): boolean {
  if (opening.form === "question" && closing.form === "question") return false;
  if (!sameTone(opening.tone, closing.tone)) return false;
  if (context && !sameTone(opening.tone, context.tone)) return false;
  return true;
}

function workloadOf(signals: Signals): "empty" | "light" | "steady" | "heavy" {
  if (signals.conversationCount <= 0) return "empty";
  if (signals.conversationCount <= 4) return "light";
  if (signals.conversationCount <= 19) return "steady";
  return "heavy";
}

export function openingCandidates(signals: Signals): readonly Fragment[] {
  // 初めての訪問でははじめましてだけを使います。時間帯の挨拶と混ぜると
  // 最初の一印象が二者択一になり、どちらかが消えてしまいます。
  if (signals.gap === "first") return firstOpenings;
  if (signals.gap === "reload" || signals.gap === "sameDay") return openings[signals.partOfDay];
  return [...returningOpenings, ...openings[signals.partOfDay]];
}

function dayKindContexts(signals: Signals): readonly Fragment[] {
  switch (signals.dayKind) {
    case "monday":
      return mondayContexts;
    case "friday":
      return fridayContexts;
    case "sunday":
      return sundayContexts;
    case "weekend":
      return weekendContexts;
    default:
      return weekdayContexts;
  }
}

function partOfDayContexts(signals: Signals): readonly Fragment[] {
  if (signals.partOfDay === "earlyMorning" || signals.partOfDay === "morning") return morningEdgeContexts;
  if (signals.partOfDay === "midday") return middayContexts;
  if (signals.partOfDay === "lateNight") return lateNightContexts;
  if (signals.partOfDay === "evening" || signals.partOfDay === "dusk") return nightContexts;
  return [];
}

/** より具体的な状況ほど重みを上げ、時刻だけの文へ落ちないようにします。 */
export function contextCandidates(signals: Signals): readonly Fragment[] {
  const specific: readonly Fragment[] = [
    ...(signals.holiday && holidayContexts[signals.holiday] ? [holidayContexts[signals.holiday]] : []),
    ...(signals.event && eventContexts[signals.event] ? [eventContexts[signals.event]] : []),
    ...(signals.solarTerm && solarTermContexts[signals.solarTerm] ? [solarTermContexts[signals.solarTerm]] : []),
    ...(signals.yearProgress === "start" ? yearStartContexts : []),
    ...(signals.yearProgress === "end" ? yearEndContexts : []),
  ];
  const desktop: readonly Fragment[] = [
    ...withWeight(signals.projectActive ? projectContexts : noProjectContexts, 2),
  ];
  const general: readonly Fragment[] = [
    ...withWeight(dayKindContexts(signals), 2),
    ...withWeight(partOfDayContexts(signals), 1),
    ...withWeight(activityContexts[signals.activity] ?? [], 1),
    ...withWeight(gapContexts[signals.gap] ?? [], 1),
    ...withWeight(workloadContexts[workloadOf(signals)] ?? [], 1),
    ...withWeight(seasonContexts[signals.season] ?? [], 1),
  ];
  return [...withWeight(specific, 4), ...desktop, ...general];
}

export function closingCandidates(signals: Signals): readonly Fragment[] {
  if (signals.activity === "yesterday" || signals.activity === "today")
    return [...resumeClosings, ...questionClosings, ...offerClosings];
  if (signals.partOfDay === "lateNight" || signals.partOfDay === "earlyMorning")
    return [...offerClosings, ...togetherClosings, ...questionClosings];
  if (signals.dayKind === "weekend" || signals.dayKind === "sunday")
    return [...togetherClosings, ...offerClosings, ...questionClosings];
  if (signals.holiday || signals.event)
    return [...offerClosings, ...questionClosings, ...togetherClosings];
  return [...questionClosings, ...offerClosings, ...togetherClosings];
}

export function noteCandidates(signals: Signals): readonly Fragment[] {
  const groups: (readonly Fragment[])[] = [];
  if (signals.gap !== "first") groups.push(withWeight(notes.returning, 4));
  if (signals.partOfDay === "lateNight" || signals.partOfDay === "evening")
    groups.push(withWeight(notes.night, 3));
  // Desktop-Agent はファイルを書き換えられるので、補足は実行モデルを訴える。
  if (signals.surface === "desktop-agent") groups.push(withWeight(notes.agent, 4));
  if (signals.projectActive) groups.push(withWeight(notes.project, 3));
  if (workloadOf(signals) === "heavy") groups.push(withWeight(notes.heavy, 3));
  if (signals.conversationCount === 0) groups.push(withWeight(notes.empty, 2));
  groups.push(notes.default);
  return groups.flat();
}

export interface Composed {
  title: string[];
  note: Fragment;
  used: string[];
}

/** 見出しを日本語の文単位に分解して返します。表示側で言語ごとに結合します。 */
export function compose(
  signals: Signals,
  recent: readonly string[] = [],
  random: () => number = Math.random,
): Composed {
  const seen = new Set(recent);
  const openingPool = openingCandidates(signals);
  const contextPool = contextCandidates(signals);
  const closingPool = closingCandidates(signals);
  const notePool = noteCandidates(signals);

  const note = pickFragment(notePool, seen, random) ?? notePool[0];
  const opening = pickFragment(openingPool, seen, random) ?? openingPool[0];
  let context = random() < CONTEXT_CHANCE ? pickFragment(contextPool, seen, random) ?? null : null;
  let closing = pickFragment(closingPool, seen, random) ?? closingPool[0];

  for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt++) {
    if (compatible(opening, context, closing))
      return {
        title: (context ? [opening, context, closing] : [opening, closing]).map((fragment) => fragment.text),
        note,
        used: [opening.id, ...(context ? [context.id] : []), closing.id, note.id],
      };
    // 文体が合わないときはまず状況節を捨て、
    // それでも合わなければ締めだけを差し替えます。
    if (context) {
      context = null;
      continue;
    }
    const next = pickFragment(closingPool, seen, random);
    if (!next || next.id === closing.id) break;
    closing = next;
  }

  // 抽選で決まらなかった場合は、プールを頭から走査して必ず組み合わせます。
  // 走査でも直近の部品を避けられるよう、未使用のものを先に試します。
  const unseenClosing = closingPool.filter((fragment) => !seen.has(fragment.id));
  const unseenContext = contextPool.filter((fragment) => !seen.has(fragment.id));
  for (const contextCandidates of [unseenContext, contextPool]) {
    for (const closingCandidates of [unseenClosing, closingPool]) {
      for (const candidateContext of [null, ...contextCandidates]) {
        for (const candidateClosing of closingCandidates) {
          if (!compatible(opening, candidateContext, candidateClosing)) continue;
          const parts = candidateContext
            ? [opening, candidateContext, candidateClosing]
            : [opening, candidateClosing];
          return {
            title: parts.map((fragment) => fragment.text),
            note,
            used: [...parts.map((fragment) => fragment.id), note.id],
          };
        }
      }
    }
  }

  // ここまで来るのは文体タグが全て外れた場合のみ。最後の一文に落とす。
  const fallback = sentence("compose-fallback", "進めたいことを教えてください。", "calm");
  return { title: [opening.text, fallback.text], note, used: [opening.id, fallback.id, note.id] };
}