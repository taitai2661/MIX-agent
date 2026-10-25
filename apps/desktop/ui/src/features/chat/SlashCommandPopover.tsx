import { memo, useEffect, useRef } from "react";
import {
  CheckCircle2,
  Eye,
  GitCommit,
  HelpCircle,
  ListChecks,
  Sparkles,
  Wrench,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";

export interface SlashCommand {
  id: string;
  name: string;
  description: string;
  prompt: string;
  icon: typeof Sparkles;
}

export const SLASH_COMMANDS: SlashCommand[] = [
  {
    id: "test",
    name: "/test",
    description: "テストを実行しエラーを自己修復",
    prompt:
      "テストを実行し、もし失敗した箇所があれば原因を分析して修正してください。",
    icon: CheckCircle2,
  },
  {
    id: "fix",
    name: "/fix",
    description: "バグや問題点を特定して修正",
    prompt:
      "発生している問題やエラーの原因を調査し、コードを修正してください。",
    icon: Wrench,
  },
  {
    id: "review",
    name: "/review",
    description: "変更差分をレビューして改善点を指摘",
    prompt:
      "現在の変更差分（git diff）を精査し、コードの品質・エッジケース・改善点をレビューしてください。",
    icon: Eye,
  },
  {
    id: "commit",
    name: "/commit",
    description: "変更内容に応じたコミットを作成",
    prompt:
      "変更内容を確認し、Conventional Commits形式で適切なコミットメッセージを作成してください。",
    icon: GitCommit,
  },
  {
    id: "plan",
    name: "/plan",
    description: "実装計画と手順をステップ別に策定",
    prompt:
      "このタスクの実装計画と手順を段階的にステップバイステップで策定してください。",
    icon: ListChecks,
  },
  {
    id: "explain",
    name: "/explain",
    description: "コードの仕様と動作をわかりやすく解説",
    prompt:
      "対象のコードや処理の動作仕様、および設計意図をわかりやすく解説してください。",
    icon: HelpCircle,
  },
];

interface SlashCommandPopoverProps {
  query: string;
  onSelect: (command: SlashCommand) => void;
  selectedIndex: number;
  setSelectedIndex: (idx: number | ((prev: number) => number)) => void;
}

export const SlashCommandPopover = memo(function SlashCommandPopover({
  query,
  onSelect,
  selectedIndex,
  setSelectedIndex,
}: SlashCommandPopoverProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  const filtered = SLASH_COMMANDS.filter(
    (cmd) =>
      cmd.name.toLowerCase().includes(query.toLowerCase()) ||
      cmd.description.toLowerCase().includes(query.toLowerCase()),
  );

  useEffect(() => {
    setSelectedIndex(0);
  }, [query, setSelectedIndex]);

  useEffect(() => {
    if (!containerRef.current) return;
    const items = containerRef.current.querySelectorAll("[data-slash-item]");
    const active = items[selectedIndex] as HTMLElement | undefined;
    if (active) {
      active.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIndex]);

  if (filtered.length === 0) return null;

  return (
    <div
      ref={containerRef}
      className="absolute bottom-full left-4 z-40 mb-2 max-h-64 w-88 overflow-y-auto rounded-xl border border-line bg-surface p-1 shadow-pop animate-scale"
      role="listbox"
      aria-label={t("クイックコマンド")}
    >
      <div className="flex items-center justify-between px-2.5 py-1.5 text-2xs font-medium text-faint border-b border-line">
        <span>{t("クイックコマンド (/)")}</span>
      </div>

      <div className="flex flex-col gap-0.5 pt-1">
        {filtered.map((cmd, idx) => {
          const isSelected = idx === selectedIndex;
          const Icon = cmd.icon;

          return (
            <button
              key={cmd.id}
              type="button"
              data-slash-item
              role="option"
              aria-selected={isSelected}
              onMouseEnter={() => setSelectedIndex(idx)}
              onClick={() => onSelect(cmd)}
              className={cn(
                "flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-xs text-start transition-colors",
                isSelected
                  ? "bg-hover text-fg font-medium"
                  : "text-muted hover:bg-hover hover:text-fg",
              )}
            >
              <span className="grid size-6 shrink-0 place-items-center rounded-md bg-inset text-brand">
                <Icon size={13} aria-hidden="true" />
              </span>
              <div className="min-w-0 flex-1 flex flex-col gap-0.5">
                <div className="flex items-center gap-2">
                  <span className="font-mono font-semibold text-fg">
                    {cmd.name}
                  </span>
                  <span className="text-2xs text-muted truncate">
                    {t(cmd.description)}
                  </span>
                </div>
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
});
