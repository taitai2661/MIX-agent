import { memo, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { FileDiff, FileText, GitBranch } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { DiffLines, parseUnifiedDiff } from "@/features/chat/DiffView";
import { useActiveRoot, useGitStatus } from "@/lib/queries";
import { ListRow, PanelHeader, PANEL_BODY } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { EmptyState, ErrorAlert, ErrorBox, ListSkeleton } from "@/components/shared";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { GitStatusBadge } from "@/components/ui/status";
import { gitStatusLabel, gitStatusLetter, gitStatusTone } from "@/lib/git";

type Scope = "unstaged" | "staged";

/** Render a `git diff` payload: split into per-file blocks for headers. */
const GitDiffBody = memo(function GitDiffBody({ unified }: { unified: string }) {
  const blocks = useMemo(
    () => unified.split(/^diff --git /m).filter((b) => b.trim()).slice(0, 50),
    [unified],
  );
  if (!unified.trim()) {
    return <EmptyState title={t("差分はありません")} />;
  }
  return (
    <div className="flex flex-col gap-2.5">
      {blocks.map((block, i) => (
        <GitDiffBlock key={i} block={block} />
      ))}
    </div>
  );
});

const GitDiffBlock = memo(function GitDiffBlock({ block }: { block: string }) {
  const label = useMemo(() => {
    const firstLine = block.split("\n")[0];
    const pathMatch = /\b(?:a|b)\/(.+?)\s+b\/(.+)$/.exec(firstLine);
    return pathMatch ? pathMatch[2] : firstLine.trim();
  }, [block]);
  const { lines, capped } = useMemo(() => {
    const parsed = parseUnifiedDiff(`diff --git ${block}`);
    return { lines: parsed.slice(0, 2000), capped: parsed.length > 2000 };
  }, [block]);
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-1.5">
        <FileDiff size={13} className="shrink-0 text-faint" />
        <span className="min-w-0 truncate font-mono text-xs text-fg">
          {label}
        </span>
      </div>
      <DiffLines lines={lines} />
      {capped && (
        <p className="text-xs text-faint">{t("大きな差分のため先頭2000行のみ表示します")}</p>
      )}
    </div>
  );
});

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
        <div className={PANEL_BODY}>
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
        <div className={PANEL_BODY}>
          {diff.isPending ? (
            <ListSkeleton rows={1} />
          ) : diff.isError ? (
            <ErrorAlert error={diff.error} />
          ) : !diff.data?.available ? (
            <ErrorBox error={diff.data?.unified_diff || t("git を利用できません")} />
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
      <div className={PANEL_BODY}>
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
          <ListSkeleton rows={1} />
        ) : status.isError ? (
          <ErrorBox error={status.error} />
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
        <div className={PANEL_BODY}>
          <EmptyState title={t("プロジェクトにフォルダを設定してください")} />
        </div>
      </>
    );
  }

  return (
    <>
      {header}
      {file.isPending ? (
        <div className={PANEL_BODY}>
          <ListSkeleton rows={1} />
        </div>
      ) : file.isError ? (
        <div className={PANEL_BODY}>
          <ErrorAlert error={file.error} />
        </div>
      ) : file.data.is_binary ? (
        <div className={PANEL_BODY}>
          <EmptyState title={t("バイナリファイル")} />
        </div>
      ) : (
        <FilePreviewBody content={file.data.content} truncated={file.data.truncated} />
      )}
    </>
  );
}

const PREVIEW_MAX_LINES = 1000;

const FilePreviewBody = memo(function FilePreviewBody({
  content,
  truncated,
}: {
  content: string;
  truncated: boolean;
}) {
  const { lines, capped } = useMemo(() => {
    const all = content.split("\n");
    return { lines: all.slice(0, PREVIEW_MAX_LINES), capped: all.length > PREVIEW_MAX_LINES };
  }, [content]);
  return (
        <div className="flex min-h-0 flex-1 flex-col">
          {(truncated || capped) && (
            <div className="shrink-0 border-b border-line bg-warning-soft px-3 py-1.5 text-xs text-warning">
              {t("ファイルが大きすぎるため冒頭のみ表示します")}
            </div>
          )}
          <div className="min-h-0 flex-1 overflow-auto">
            {lines.map((line, i) => (
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
  );
});
