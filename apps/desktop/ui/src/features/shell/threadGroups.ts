import type { ThreadGroup } from "@/lib/format";

export type ExtendedThreadGroup = "pinned" | ThreadGroup;

export const GROUP_ORDER: ExtendedThreadGroup[] = [
  "pinned",
  "today",
  "yesterday",
  "week",
  "older",
];

export const GROUP_LABEL: Record<ExtendedThreadGroup, string> = {
  pinned: "ピン留め済み",
  today: "今日",
  yesterday: "昨日",
  week: "過去7日",
  older: "それより前",
};

/** Run modes, shown as words rather than the wire values. */
export const MODE_LABEL: Record<string, string> = {
  chat: "チャット",
  thinking: "思考",
  agent: "自動実行",
  plan: "計画",
};
