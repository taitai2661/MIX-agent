import { memo, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  Check,
  ChevronRight,
  CircleAlert,
  ExternalLink,
  FileCode,
  GitBranch,
  Globe,
  Search,
  Terminal as TerminalIcon,
  Undo2,
  Wrench,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t, tf } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { useActiveRoot } from "@/lib/queries";
import { describeToolCall, type ToolActivity } from "@/lib/runEvents";
import { Spinner } from "@/components/ui/spinner";
import { Badge } from "@/components/ui/badge";
import { ErrorBadge, payloadBlock } from "@/components/shared";
import { TerminalResultView } from "./TerminalResultView";
import { buildDiffFromArgs } from "./ApprovalDiffView";
import { DiffLines, parseUnifiedDiff } from "./DiffView";

interface InlineToolItemProps {
  tool: ToolActivity;
  defaultOpen?: boolean;
}

function toolIcon(name: string) {
  switch (name) {
    case "terminal":
      return TerminalIcon;
    case "edit":
    case "files":
      return FileCode;
    case "git":
      return GitBranch;
    case "browser":
      return Globe;
    case "symbols":
    case "glob":
    case "grep":
    case "search":
      return Search;
    default:
      return Wrench;
  }
}

export const InlineToolItem = memo(function InlineToolItem({
  tool,
  defaultOpen = false,
}: InlineToolItemProps) {
  const [open, setOpen] = useState(defaultOpen);
  const Icon = toolIcon(tool.name);

  // Parse diff for edit / files tools
  const diffData = useMemo(() => {
    if (tool.name !== "edit" && tool.name !== "files") return null;

    // Check if result has unified diff preview (e.g. from Rust EditOutcome)
    if (
      tool.result &&
      typeof tool.result === "object" &&
      "preview" in tool.result &&
      typeof (tool.result as { preview: unknown }).preview === "string"
    ) {
      const preview = (tool.result as { preview: string }).preview;
      return { unified: preview };
    }

    // Otherwise build from arguments
    if (tool.arguments && typeof tool.arguments === "object") {
      return buildDiffFromArgs(
        tool.name,
        tool.arguments as Record<string, unknown>,
      );
    }
    return null;
  }, [tool.name, tool.arguments, tool.result]);

  const parsedDiffLines = useMemo(() => {
    if (!diffData?.unified) return null;
    return parseUnifiedDiff(diffData.unified);
  }, [diffData]);

  const queryClient = useQueryClient();
  const activeRoot = useActiveRoot();
  const setRightPanel = useUi((s) => s.setRightPanel);
  const setGitDiffPath = useUi((s) => s.setGitDiffPath);
  const openBrowserPanel = useUi((s) => s.openBrowserPanel);
  const [busy, setBusy] = useState(false);

  const handleOpenInChanges = (path: string) => {
    setGitDiffPath(path);
    setRightPanel("changes");
  };

  const handleDiscard = async (path: string) => {
    if (!activeRoot) return;
    if (!window.confirm(tf("「{path}」の変更を元に戻しますか？", { path })))
      return;
    setBusy(true);
    try {
      await api.gitRestore(activeRoot, path);
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["git", "status", activeRoot],
        }),
        queryClient.invalidateQueries({
          queryKey: ["git", "diff", activeRoot],
        }),
        queryClient.invalidateQueries({
          queryKey: ["files", "tree", activeRoot],
        }),
      ]);
    } catch (err) {
      console.error("Failed to discard file:", err);
    } finally {
      setBusy(false);
    }
  };

  const isTerminal = tool.name === "terminal";
  const hasDetail =
    tool.arguments != null ||
    tool.result != null ||
    tool.error != null ||
    parsedDiffLines != null;

  return (
    <details
      open={open}
      onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}
      className="group rounded-lg border border-line bg-surface/90 text-xs shadow-xs transition-colors hover:border-line-strong"
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 px-2.5 py-1.5 transition-colors hover:bg-hover">
        <Icon size={13} className="shrink-0 text-faint" aria-hidden="true" />
        <span className="shrink-0 font-mono font-medium text-fg">
          {tool.name}
        </span>
        <span
          className="min-w-0 flex-1 truncate text-muted"
          title={describeToolCall(tool)}
        >
          {describeToolCall(tool)}
        </span>

        {tool.status === "running" ? (
          <span className="flex items-center gap-1 shrink-0 text-2xs text-brand">
            <Spinner size={10} />
            <span className="font-sans">{t("実行中…")}</span>
          </span>
        ) : tool.status === "error" ? (
          <Badge tone="danger" size="sm" className="shrink-0">
            <CircleAlert size={10} aria-hidden="true" />
            <span>{t("失敗")}</span>
          </Badge>
        ) : (
          <Badge tone="neutral" size="sm" className="shrink-0">
            <Check size={10} className="text-success" aria-hidden="true" />
            <span>{t("完了")}</span>
          </Badge>
        )}

        {hasDetail && (
          <ChevronRight
            size={12}
            className="shrink-0 text-faint transition-transform group-open:rotate-90"
            aria-hidden="true"
          />
        )}
      </summary>

      {hasDetail && (
        <div className="flex flex-col gap-2 border-t border-line bg-inset/40 p-2.5">
          {tool.name === "browser" && (
            <div className="flex items-center gap-1">
              <button
                type="button"
                onClick={() => openBrowserPanel("user")}
                className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-2xs text-muted hover:bg-hover hover:text-fg"
                title={t("ブラウザ画面を開く")}
              >
                <Globe size={10} />
                <span className="font-sans">{t("ブラウザ画面")}</span>
              </button>
            </div>
          )}
          {tool.error && <ErrorBadge error={tool.error} />}

          {/* Rich Terminal view */}
          {isTerminal && tool.result && typeof tool.result === "object" ? (
            <TerminalResultView
              result={tool.result as Record<string, unknown>}
            />
          ) : parsedDiffLines && parsedDiffLines.length > 0 ? (
            /* Rich Inline Diff view for file edits */
            <div className="flex flex-col gap-1.5">
              <div className="flex items-center justify-between gap-2 font-mono text-2xs text-muted">
                {diffData?.path ? (
                  <div className="flex min-w-0 items-center gap-1.5">
                    <FileCode size={11} className="shrink-0 text-brand" />
                    <span className="truncate font-medium text-fg">
                      {diffData.path}
                    </span>
                  </div>
                ) : (
                  <span />
                )}
                {diffData?.path && (
                  <div className="flex items-center gap-1">
                    <button
                      type="button"
                      onClick={() => handleOpenInChanges(diffData.path!)}
                      className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-2xs text-muted hover:bg-hover hover:text-fg"
                      title={t("差分パネルで開く")}
                    >
                      <ExternalLink size={10} />
                      <span className="font-sans">{t("差分パネル")}</span>
                    </button>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => handleDiscard(diffData.path!)}
                      className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-2xs text-danger hover:bg-danger/10 disabled:opacity-50"
                      title={t("変更を破棄")}
                    >
                      <Undo2 size={10} />
                      <span className="font-sans">{t("破棄")}</span>
                    </button>
                  </div>
                )}
              </div>
              <div className="max-h-72 overflow-y-auto rounded-lg border border-line bg-surface">
                <DiffLines lines={parsedDiffLines} />
              </div>
            </div>
          ) : (
            /* Generic arguments and result display */
            <div className="flex flex-col gap-1.5">
              {tool.arguments != null && (
                <div>
                  <div className="mb-0.5 text-2xs font-medium text-faint">
                    {t("引数")}
                  </div>
                  <pre className={cn("max-h-40 overflow-y-auto", payloadBlock)}>
                    {JSON.stringify(tool.arguments, null, 2)}
                  </pre>
                </div>
              )}
              {tool.result != null && (
                <div>
                  <div className="mb-0.5 text-2xs font-medium text-faint">
                    {t("結果")}
                  </div>
                  <pre className={cn("max-h-40 overflow-y-auto", payloadBlock)}>
                    {JSON.stringify(tool.result, null, 2)}
                  </pre>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </details>
  );
});
