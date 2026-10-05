import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronDown,
  ChevronRight,
  Eye,
  EyeOff,
  File,
  FileText,
  Folder,
  FolderOpen,
  FolderTree,
  RefreshCw,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { useActiveProject, gitStatusKey, useGitStatus } from "@/lib/queries";
import { PanelHeader } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { GitStatusBadge } from "@/components/ui/status";
import { ErrorAlert } from "@/components/shared";
import { gitStatusLabel, gitStatusLetter, gitStatusTone } from "@/lib/git";
import { ResizableHandle } from "@/features/shell/ResizableHandle";
import { panelStyle, panelWidth } from "@/features/shell/usePanelCap";
import { usePanelWidth } from "@/features/shell/usePanelWidth";

const TEXT_EXTS = new Set([
  "txt",
  "md",
  "rst",
  "json",
  "yaml",
  "yml",
  "toml",
  "ini",
  "cfg",
  "csv",
  "tsv",
  "rs",
  "ts",
  "tsx",
  "js",
  "jsx",
  "py",
  "go",
  "java",
  "c",
  "h",
  "cpp",
  "hpp",
  "css",
  "scss",
  "html",
  "svg",
  "sh",
  "bash",
  "zsh",
  "sql",
  "xml",
  "env",
  "lock",
]);

function isTextFile(name: string): boolean {
  const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
  return TEXT_EXTS.has(ext) || !name.includes(".");
}

/**
 * Row styling for the tree. Real `<button>`s now, not `div role="button"` with
 * hand-rolled Enter/Space handling. The height comes from the shared `--row`
 * token so the tree matches every other list in the app; it used to be `h-6`,
 * a fourth density alongside the sidebar's two-line rows and the palette's
 * `h-8`.
 */
const rowClass = [
  "flex h-row w-full cursor-pointer items-center gap-1.5 rounded-lg",
  "pe-1.5 ps-1.5 text-start text-xs text-muted",
  "transition-colors duration-75 hover:bg-hover hover:text-fg",
].join(" ");

interface TreeContext {
  root: string;
  expanded: ReadonlySet<string>;
  toggle: (path: string) => void;
  gitMap: ReadonlyMap<string, string>;
  showHidden: boolean;
  onOpenFile: (path: string) => void;
}

function DirRows({
  path,
  depth,
  ctx,
}: {
  path: string;
  depth: number;
  ctx: TreeContext;
}) {
  const query = useQuery({
    queryKey: ["dir", ctx.root, path],
    queryFn: () => api.listDir(ctx.root, path),
    staleTime: 10_000,
  });

  // A placeholder is not a control, so it deliberately drops the row's
  // hover/focus affordance — the shared `rowClass` used to leave both in place,
  // which made a loading row look clickable.
  const placeholder = (text: string) => (
    <div
      className="flex h-row w-full items-center gap-1.5 rounded-lg ps-1.5 pe-1.5 text-xs"
      style={{ paddingLeft: `${8 + depth * 14}px` }}
    >
      <span className="truncate text-faint">{text}</span>
    </div>
  );

  if (query.isPending) return placeholder(t("読み込み中…"));
  if (query.isError)
    return placeholder(errorMessage(query.error) || t("エラー"));

  const entries = (query.data ?? []).filter(
    (entry) => ctx.showHidden || !entry.hidden,
  );
  if (entries.length === 0) return placeholder(t("（空）"));

  return (
    <>
      {entries.map((entry) => {
        const childPath = path ? `${path}/${entry.name}` : entry.name;
        const indent = { paddingLeft: `${6 + depth * 14}px` };

        if (entry.is_dir) {
          const open = ctx.expanded.has(childPath);
          return (
            <div key={childPath}>
              <button
                type="button"
                className={rowClass}
                style={indent}
                aria-expanded={open}
                onClick={() => ctx.toggle(childPath)}
              >
                {open ? (
                  <ChevronDown size={12} className="shrink-0" />
                ) : (
                  <ChevronRight size={12} className="shrink-0" />
                )}
                {open ? (
                  <FolderOpen size={13} className="shrink-0 text-faint" />
                ) : (
                  <Folder size={13} className="shrink-0 text-faint" />
                )}
                <span className="truncate">{entry.name}</span>
              </button>
              {open && <DirRows path={childPath} depth={depth + 1} ctx={ctx} />}
            </div>
          );
        }

        const status = ctx.gitMap.get(childPath);
        const icon = isTextFile(entry.name) ? (
          <File size={13} className="shrink-0 text-faint" />
        ) : (
          <FileText size={13} className="shrink-0 text-faint" />
        );
        return (
          <button
            key={childPath}
            type="button"
            className={rowClass}
            style={indent}
            onClick={() => ctx.onOpenFile(childPath)}
            title={
              status ? `${entry.name} — ${gitStatusLabel(status)}` : entry.name
            }
          >
            <span className="w-3 shrink-0" />
            {icon}
            <span className="truncate">{entry.name}</span>
            {status && (
              <GitStatusBadge
                letter={gitStatusLetter(status)}
                label={gitStatusLabel(status)}
                tone={gitStatusTone(status)}
              />
            )}
          </button>
        );
      })}
    </>
  );
}

export function FileTreePane({ cap = null }: { cap?: number | null }) {
  const project = useActiveProject();
  const root = project?.rootPath ?? null;
  const toggleFileTree = useUi((s) => s.toggleFileTree);
  const openFilePreview = useUi((s) => s.openFilePreview);
  const setView = useUi((s) => s.setView);
  const queryClient = useQueryClient();
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [showHidden, setShowHidden] = useState(false);
  const { width, setWidth, commitWidth } = usePanelWidth(
    "mix-agent-file-tree-width",
    240,
    180,
    480,
  );

  const git = useGitStatus(root);

  const gitMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const entry of git.data?.entries ?? [])
      map.set(entry.path, entry.status);
    return map;
  }, [git.data]);

  const ctx: TreeContext = {
    root: root ?? "",
    expanded,
    toggle: (path) =>
      setExpanded((prev) => {
        const next = new Set(prev);
        if (next.has(path)) next.delete(path);
        else next.add(path);
        return next;
      }),
    gitMap,
    showHidden,
    onOpenFile: (path) => openFilePreview(path),
  };

  const refresh = () => {
    if (!root) return;
    void queryClient.invalidateQueries({ queryKey: ["dir", root] });
    void queryClient.invalidateQueries({ queryKey: gitStatusKey(root ?? "") });
  };

  return (
    <>
      <aside
        aria-label={t("ファイルツリー")}
        data-side-panel=""
        style={panelStyle(width, cap)}
        className="flex shrink-0 flex-col overflow-hidden border-e border-line bg-sidebar"
      >
        <PanelHeader
          className="px-2.5"
          icon={<FolderTree size={13} />}
          title={project ? project.name : t("プロジェクト")}
          titleClass="text-xs font-semibold"
          onClose={toggleFileTree}
          trailing={
            <>
              {git.data?.available && git.data.branch && (
                // Branch names are as long as the team wants them to be, and the
                // title next to them only has `flex-1`. `shrink-0` with no width
                // of its own meant a long branch pushed the header — and the
                // panel's right edge — out of the window, so the badge gives up
                // width instead and the name goes to an ellipsis.
                <Badge
                  tone="neutral"
                  size="sm"
                  className="min-w-0 max-w-28 shrink font-mono"
                  title={git.data.branch}
                >
                  <span className="min-w-0 truncate">{git.data.branch}</span>
                </Badge>
              )}
              <Button
                variant="ghost"
                size="icon-sm"
                title={
                  showHidden ? t("隠しファイルを隠す") : t("隠しファイルを表示")
                }
                aria-label={
                  showHidden ? t("隠しファイルを隠す") : t("隠しファイルを表示")
                }
                onClick={() => setShowHidden((v) => !v)}
              >
                {showHidden ? <Eye size={12} /> : <EyeOff size={12} />}
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                title={t("更新")}
                aria-label={t("更新")}
                onClick={refresh}
              >
                <RefreshCw size={12} />
              </Button>
            </>
          }
        />

        {root ? (
          <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-1.5">
            <DirRows path="" depth={0} ctx={ctx} />
          </div>
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 p-4 text-center">
            <p className="m-0 text-xs text-faint">
              {t("プロジェクトにフォルダを設定してください")}
            </p>
            <Button size="sm" onClick={() => setView("projects")}>
              {t("プロジェクト")}
            </Button>
          </div>
        )}

        {root && git.isError && (
          <div className="p-2">
            <ErrorAlert error={t("git を利用できません")} />
          </div>
        )}
      </aside>
      <ResizableHandle
        value={panelWidth(width, cap)}
        min={180}
        max={480}
        label={t("ファイルツリーの幅")}
        onChange={setWidth}
        onCommit={commitWidth}
      />
    </>
  );
}
