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
import type { ComposerMode } from "./composerTypes";

export interface SlashCommand {
  id: string;
  name: string;
  description: string;
  prompt: string;
  icon: typeof Sparkles;
  /** Run mode this command switches to before submitting its prompt. */
  mode?: ComposerMode;
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
    // Switches the composer to the engine's read-only plan mode, so the model
    // investigates first and presents the result through `user_plan` instead of
    // editing files.
    mode: "plan",
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
  /** Modes the active surface exposes; commands that need another mode hide. */
  allowedModes?: ReadonlyArray<ComposerMode>;
}

export const SlashCommandPopover = memo(function SlashCommandPopover({
  query,
  onSelect,
  selectedIndex,
  setSelectedIndex,
  allowedModes,
}: SlashCommandPopoverProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  const visible = SLASH_COMMANDS.filter(
    (cmd) =>
      cmd.mode === undefined ||
      !allowedModes ||
      allowedModes.length === 0 ||
      allowedModes.includes(cmd.mode),
  );
  const filtered = visible.filter(
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
    // Optional call: jsdom does not implement scrollIntoView, and a missing
    // method used to throw inside the effect whenever the list scrolled.
    active?.scrollIntoView?.({ block: "nearest" });
  }, [selectedIndex]);

  if (filtered.length === 0) return null;

  return (
    <div
      ref={containerRef}
      className="absolute inset-x-0 bottom-full z-40 mb-2 flex max-h-[min(26rem,70vh)] flex-col overflow-y-auto rounded-2xl border border-line bg-surface p-1.5 shadow-pop animate-scale"
      role="listbox"
      aria-label={t("クイックコマンド")}
    >
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
              "flex h-11 w-full shrink-0 cursor-pointer items-center gap-3 rounded-xl px-3 text-start transition-colors duration-100",
              isSelected ? "bg-hover" : "hover:bg-hover/60",
            )}
          >
            <Icon
              size={18}
              strokeWidth={1.8}
              className="shrink-0 text-muted"
              aria-hidden="true"
            />
            <span className="shrink-0 text-sm font-medium text-fg">
              {cmd.name}
            </span>
            <span className="min-w-0 truncate text-xs text-muted">
              {t(cmd.description)}
            </span>
          </button>
        );
      })}
    </div>
  );
});
