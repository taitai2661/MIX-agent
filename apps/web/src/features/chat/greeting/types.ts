export type PartOfDay =
  | "earlyMorning"
  | "morning"
  | "lateMorning"
  | "midday"
  | "afternoon"
  | "dusk"
  | "evening"
  | "lateNight";

export type DayKind = "sunday" | "weekend" | "monday" | "friday" | "weekday";

export type Gap =
  | "first"
  | "reload"
  | "sameDay"
  | "overnight"
  | "fewDays"
  | "longAway"
  | "farAway";

export type Activity =
  | "none"
  | "justNow"
  | "today"
  | "yesterday"
  | "thisWeek"
  | "earlier";

export type Workload = "empty" | "light" | "steady" | "heavy";

export type Season = "spring" | "summer" | "autumn" | "winter";

export type SeasonPhase = "early" | "mid" | "late";

export type YearProgress = "start" | "end" | null;

export type Tone = "polite" | "calm" | "casual";

export type Form = "question" | "declarative";

export interface Fragment {
  id: string;
  text: string;
  tone: Tone;
  form: Form;
  weight?: number;
}

export interface Signals {
  now: Date;
  partOfDay: PartOfDay;
  dayKind: DayKind;
  season: Season;
  seasonPhase: SeasonPhase;
  solarTerm: string | null;
  holiday: string | null;
  event: string | null;
  yearProgress: YearProgress;
  conversationCount: number;
  pinnedCount: number;
  projectCount: number;
  agentCount: number;
  activity: Activity;
  gap: Gap;
  visits: number;
  firstToday: boolean;
}

export interface Suggestion {
  id: string;
  label: string;
  prompt: string;
}

export interface Welcome {
  id: string;
  title: string[];
  /** `title` と同じ並びの部品 ID。翻訳はこの ID で引く。 */
  titleIds: string[];
  note: string;
  /** 補足文の部品 ID。翻訳はこの ID で引く。 */
  noteId: string;
  suggestions: Suggestion[];
  /** 直近の文面で使った部品の ID。次回の抽選で除外します。 */
  used: string[];
}
