import { Bot, Brain, ClipboardList, MessageSquare } from "lucide-react";
import type { ComposerMode } from "./composerTypes";

export interface ModeMeta {
  id: ComposerMode;
  label: string;
  hint: string;
  Icon: typeof MessageSquare;
}

export const ALL_MODES: ModeMeta[] = [
  // Labels stay as raw i18n keys; `t()` is applied at render time so a
  // locale switch updates them without re-importing the module.
  {
    id: "chat",
    label: "チャット",
    hint: "素早い質疑応答",
    Icon: MessageSquare,
  },
  { id: "thinking", label: "思考", hint: "深く考えて回答", Icon: Brain },
  {
    id: "agent",
    label: "自動実行",
    hint: "ツールで自律的に作業",
    Icon: Bot,
  },
  {
    id: "plan",
    label: "計画",
    hint: "読み取り専用で計画を提示",
    Icon: ClipboardList,
  },
];
