import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, FileDiff, FileText, GitBranch } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { DiffLines, parseUnifiedDiff } from "@/features/chat/DiffView";
import { useActiveRoot, useGitStatus } from "@/lib/queries";
import { ListRow, PanelHeader } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { Alert, Skeleton } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { EmptyState, ErrorAlert } from "@/components/shared";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { GitStatusBadge } from "@/components/ui/status";
import { gitStatusLabel, gitStatusLetter, gitStatusTone } from "@/lib/git";

type Scope = "unstaged" | "staged";

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
              <span className="min-w-0 truncate font-mono text-xs text-fg">
                {label}
              </span>
            </div>
            <DiffLines lines={parseUnifiedDiff(`diff --git ${block}`)} />
          </div>
        );
      })}
    </div>
  );
}

/* `[&>*]:shrink-0`: this column has a definite height and scrolls, so its
   children are flex items that would otherwise shrink to fit it — anything with
   `overflow: hidden` among them gets silently cut off instead of scrolled. */
const panelBody =
  "flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3 [&>*]:shrink-0";

/** Right-panel list of git changes with a per-file diff drill-down. */
export function ChangesPanel({
  embedded = false,
}: { embedded?: boolean } = {}) {
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
        {!embedded && (
          <PanelHeader
            icon={<GitBranch size={14} />}
            title={t("変更")}
            onClose={close}
          />
        )}
        <div className={panelBody}>
          <EmptyState title={t("プロジェクトにフォルダを設定してください")} />
        </div>
      </>
    );
  }

  if (gitDiffPath) {
    return (
      <>
        {embedded ? (
          <div className="flex h-9 shrink-0 items-center gap-1.5 border-b border-line px-2.5">
            <Button
              variant="ghost"
              size="icon-sm"
              title={t("戻る")}
              aria-label={t("戻る")}
              onClick={() => setGitDiffPath(null)}
            >
              <ArrowLeft size={13} />
            </Button>
            <span
              className="min-w-0 flex-1 truncate font-mono text-xs text-fg"
              title={gitDiffPath}
            >
              {gitDiffPath}
            </span>
          </div>
        ) : (
          <PanelHeader
            icon={<GitBranch size={14} />}
            title={gitDiffPath}
            onClose={close}
            onBack={() => setGitDiffPath(null)}
          />
        )}
        <div className={panelBody}>
          {diff.isPending ? (
            <Skeleton className="h-40 w-full" />
          ) : diff.isError ? (
            <ErrorAlert error={diff.error} />
          ) : !diff.data?.available ? (
            <Alert tone="danger">
              {diff.data?.unified_diff || t("git を利用できません")}
            </Alert>
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
      {!embedded && (
        <PanelHeader
          icon={<GitBranch size={14} />}
          title={t("変更")}
          onClose={close}
        />
      )}
      <div className={panelBody}>
        <div className="flex min-w-0 items-center gap-1.5">
          {/* Which set of changes is listed, not a panel switch: a radiogroup,
              matching every other filter in the app. */}
          <Segmented aria-label={t("変更")} className="min-w-0 flex-1">
            <SegmentedOption
              value="unstaged"
              label={t("作業ツリー")}
              selected={scope === "unstaged"}
              onSelect={setScope}
            >
              {t("作業ツリー")}
            </SegmentedOption>
            <SegmentedOption
              value="staged"
              label={t("ステージ済み")}
              selected={scope === "staged"}
              onSelect={setScope}
            >
              {t("ステージ済み")}
            </SegmentedOption>
          </Segmented>
          {status.data?.branch && (
            <Badge
              size="sm"
              className="min-w-0 max-w-28 shrink font-mono"
              title={status.data.branch}
            >
              <span className="min-w-0 truncate">{status.data.branch}</span>
            </Badge>
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
                <ListRow onClick={() => setGitDiffPath(entry.path)}>
                  <GitStatusBadge
                    letter={gitStatusLetter(entry.status)}
                    label={gitStatusLabel(entry.status)}
                    tone={gitStatusTone(entry.status)}
                  />
                  <span className="min-w-0 flex-1 truncate font-mono text-xs text-fg">
                    {entry.path}
                  </span>
                  <span className="shrink-0 text-xs text-faint">
                    {gitStatusLabel(entry.status)}
                  </span>
                </ListRow>
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
    <PanelHeader
      icon={<FileText size={14} />}
      title={path ?? ""}
      onClose={close}
    />
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
            <div className="shrink-0 border-b border-line bg-warning-soft px-3 py-1.5 text-xs text-warning">
              {t("ファイルが大きすぎるため冒頭のみ表示します")}
            </div>
          )}
          <div className="min-h-0 flex-1 overflow-auto">
            {file.data.content.split("\n").map((line, i) => (
              <div key={i} className="flex font-mono text-xs leading-5">
                <span className="sticky start-0 w-11 shrink-0 border-e border-line bg-inset px-1.5 text-end text-faint select-none">
                  {i + 1}
                </span>
                <span className="whitespace-pre-wrap break-all px-2 text-fg">
                  {line}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </>
  );
}
