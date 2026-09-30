import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { FileDiff, FileText, GitBranch } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { DiffLines, parseUnifiedDiff } from "@/features/chat/DiffView";
import { useActiveRoot, useGitStatus } from "@/lib/queries";
import { PanelHeader } from "@/components/layout";
import { Alert, Skeleton } from "@/components/ui/alert";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { EmptyState, ErrorAlert } from "@/components/shared";

type Scope = "unstaged" | "staged";

function statusLabel(status: string): string {
  if (status.includes("?")) return t("未追跡");
  if (status.includes("A")) return t("追加");
  if (status.includes("D")) return t("削除");
  if (status.includes("R")) return t("改名");
  if (status.includes("C")) return t("コピー");
  return t("変更");
}

function statusTone(status: string): NonNullable<BadgeProps["tone"]> {
  if (status.includes("?")) return "info";
  if (status.includes("D")) return "danger";
  if (status.includes("A")) return "success";
  return "warning";
}

function statusLetter(status: string): string {
  if (status.includes("?")) return "?";
  return status.trim().slice(-1) || "M";
}

/** Render a `git diff` payload: split into per-file blocks for headers. */
function GitDiffBody({ unified }: { unified: string }) {
  if (!unified.trim()) {
    return <EmptyState title={t("差分はありません")} />;
  }
  const blocks = unified.split(/^diff --git /m).filter((b) => b.trim());
  return (
    <div className="flex flex-col gap-2.5">
      {blocks.map((block, i) => {
        const firstLine = block.split("\n")[0];
        const pathMatch = /\b(?:a|b)\/(.+?)\s+b\/(.+)$/.exec(firstLine);
        const label = pathMatch ? pathMatch[2] : firstLine.trim();
        return (
          <div key={i} className="flex flex-col gap-1.5">
            <div className="flex items-center gap-1.5">
              <FileDiff size={12} className="shrink-0 text-faint" />
              <span className="min-w-0 truncate font-mono text-xs text-fg">{label}</span>
            </div>
            <DiffLines lines={parseUnifiedDiff(`diff --git ${block}`)} />
          </div>
        );
      })}
    </div>
  );
}

const panelBody = "flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3";

/** Right-panel list of git changes with a per-file diff drill-down. */
export function ChangesPanel() {
  const root = useActiveRoot();
  const setRightPanel = useUi((s) => s.setRightPanel);
  const gitDiffPath = useUi((s) => s.gitDiffPath);
  const setGitDiffPath = useUi((s) => s.setGitDiffPath);
  const [scope, setScope] = useState<Scope>("unstaged");

  const status = useGitStatus(root);

  const diff = useQuery({
    queryKey: ["git_diff", root, scope, gitDiffPath],
    queryFn: () => api.gitDiff(root!, scope, gitDiffPath),
    enabled: !!root && !!gitDiffPath,
    staleTime: 5_000,
  });

  const close = () => setRightPanel("activity");

  if (!root) {
    return (
      <>
        <PanelHeader icon={<GitBranch size={14} />} title={t("変更")} onClose={close} />
        <div className={panelBody}>
          <EmptyState title={t("プロジェクトにフォルダを設定してください")} />
        </div>
      </>
    );
  }

  if (gitDiffPath) {
    return (
      <>
        <PanelHeader
          icon={<GitBranch size={14} />}
          title={gitDiffPath}
          onClose={close}
          onBack={() => setGitDiffPath(null)}
        />
        <div className={panelBody}>
          {diff.isPending ? (
            <Skeleton className="h-40 w-full" />
          ) : diff.isError ? (
            <ErrorAlert error={diff.error} />
          ) : !diff.data?.available ? (
            <Alert tone="danger">{diff.data?.unified_diff || t("git を利用できません")}</Alert>
          ) : diff.data.unified_diff.trim() ? (
            <GitDiffBody unified={diff.data.unified_diff} />
          ) : (
            <EmptyState
              title={t("差分はありません")}
              sub={t("未追跡ファイルは差分に含まれません")}
            />
          )}
        </div>
      </>
    );
  }

  const entries = status.data?.entries ?? [];
  return (
    <>
      <PanelHeader icon={<GitBranch size={14} />} title={t("変更")} onClose={close} />
      <div className={panelBody}>
        <div className="flex items-center gap-1.5">
          <div
            role="tablist"
            aria-label={t("変更")}
            className="inline-flex items-center gap-0.5 rounded-lg bg-inset p-0.5"
          >
            {(["unstaged", "staged"] as const).map((s) => (
              <button
                key={s}
                type="button"
                role="tab"
                aria-selected={scope === s}
                onClick={() => setScope(s)}
                className={cn(
                  "h-6 rounded-md px-2 text-xs font-medium",
                  "text-muted transition-colors duration-100 hover:text-fg",
                  scope === s && "bg-surface text-fg shadow-xs",
                )}
              >
                {s === "unstaged" ? t("作業ツリー") : t("ステージ済み")}
              </button>
            ))}
          </div>
          <span className="flex-1" />
          {status.data?.branch && (
            <Badge size="sm" className="font-mono">{status.data.branch}</Badge>
          )}
        </div>

        {status.isPending ? (
          <Skeleton className="h-32 w-full" />
        ) : !status.data?.available ? (
          <EmptyState title={t("git リポジトリではありません")} />
        ) : entries.length === 0 ? (
          <EmptyState title={t("変更はありません")} />
        ) : (
          <ul className="flex flex-col gap-0.5">
            {entries.map((entry) => (
              <li key={entry.path}>
                <button
                  type="button"
                  onClick={() => setGitDiffPath(entry.path)}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-start",
                    "transition-colors duration-75 hover:bg-hover",
                    "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-focus",
                  )}
                >
                  <Badge tone={statusTone(entry.status)} size="sm" className="shrink-0 font-mono">
                    {statusLetter(entry.status)}
                  </Badge>
                  <span className="min-w-0 flex-1 truncate font-mono text-xs text-fg">
                    {entry.path}
                  </span>
                  <span className="shrink-0 text-2xs text-faint">{statusLabel(entry.status)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

/** Read-only preview of one project file. */
export function FilePreviewPanel() {
  const root = useActiveRoot();
  const path = useUi((s) => s.filePreviewPath);
  const setRightPanel = useUi((s) => s.setRightPanel);

  const file = useQuery({
    queryKey: ["workspace_file", root, path],
    queryFn: () => api.readWorkspaceFile(root!, path!),
    enabled: !!root && !!path,
    staleTime: 10_000,
  });

  const close = () => setRightPanel("activity");
  const header = (
    <PanelHeader icon={<FileText size={14} />} title={path ?? ""} onClose={close} />
  );

  if (!root || !path) {
    return (
      <>
        {header}
        <div className={panelBody}>
          <EmptyState title={t("プロジェクトにフォルダを設定してください")} />
        </div>
      </>
    );
  }

  return (
    <>
      {header}
      {file.isPending ? (
        <div className={panelBody}>
          <Skeleton className="h-40 w-full" />
        </div>
      ) : file.isError ? (
        <div className={panelBody}>
          <ErrorAlert error={file.error} />
        </div>
      ) : file.data.is_binary ? (
        <div className={panelBody}>
          <EmptyState title={t("バイナリファイル")} />
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">
          {file.data.truncated && (
            <div className="shrink-0 border-b border-line bg-warning-soft px-3 py-1.5 text-2xs text-warning">
              {t("ファイルが大きすぎるため冒頭のみ表示します")}
            </div>
          )}
          <div className="min-h-0 flex-1 overflow-auto">
            {file.data.content.split("\n").map((line, i) => (
              <div key={i} className="flex font-mono text-xs leading-5">
                <span className="sticky start-0 w-11 shrink-0 border-e border-line bg-inset px-1.5 text-end text-faint select-none">
                  {i + 1}
                </span>
                <span className="whitespace-pre-wrap break-all px-2 text-fg">{line}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </>
  );
}
