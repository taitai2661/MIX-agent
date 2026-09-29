import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronDown, ChevronRight, Eye, EyeOff, File, FileText, Folder, FolderOpen,
  FolderTree, RefreshCw, X,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { useActiveProject } from "./useActiveProject";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ResizableHandle } from "@/features/shell/ResizableHandle";
import { usePanelWidth } from "@/features/shell/usePanelWidth";

const TEXT_EXTS = new Set([
  "txt", "md", "rst", "json", "yaml", "yml", "toml", "ini", "cfg", "csv", "tsv",
  "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "c", "h", "cpp", "hpp",
  "css", "scss", "html", "svg", "sh", "bash", "zsh", "sql", "xml", "env", "lock",
]);

/**
 * "Modified" is the most common git state, so it gets the strongest of the four
 * semantic tones; the old stylesheet gave it the accent colour, which in light
 * mode was plain black and therefore indistinguishable from the file name.
 */
function badgeTone(status: string): NonNullable<BadgeProps["tone"]> {
  if (status.includes("?")) return "info";
  if (status.includes("D")) return "danger";
  if (status.includes("A")) return "success";
  return "warning";
}

function badgeLetter(status: string): string {
  if (status.includes("?")) return "?";
  return status.trim().slice(-1) || "M";
}

function isTextFile(name: string): boolean {
  const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
  return TEXT_EXTS.has(ext) || !name.includes(".");
}

const rowClass = [
  "flex h-6 w-full cursor-pointer items-center gap-1.5 rounded-md",
  "pr-1.5 text-start text-xs text-muted",
  "transition-colors duration-75 hover:bg-hover hover:text-fg",
  "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-focus",
].join(" ");

interface TreeContext {
  root: string;
  expanded: ReadonlySet<string>;
  toggle: (path: string) => void;
  gitMap: ReadonlyMap<string, string>;
  showHidden: boolean;
  onOpenFile: (path: string) => void;
}

function DirRows({ path, depth, ctx }: { path: string; depth: number; ctx: TreeContext }) {
  const query = useQuery({
    queryKey: ["dir", ctx.root, path],
    queryFn: () => api.listDir(ctx.root, path),
    staleTime: 10_000,
  });

  const placeholder = (text: string) => (
    <div className={cn(rowClass, "cursor-default")} style={{ paddingLeft: `${8 + depth * 14}px` }}>
      <span className="truncate text-faint">{text}</span>
    </div>
  );

  if (query.isPending) return placeholder(t("読み込み中…"));
  if (query.isError) return placeholder(errorMessage(query.error) || t("エラー"));

  const entries = (query.data ?? []).filter((entry) => ctx.showHidden || !entry.hidden);
  if (entries.length === 0) return placeholder("（空）");

  return (
    <>
      {entries.map((entry) => {
        const childPath = path ? `${path}/${entry.name}` : entry.name;
        const indent = { paddingLeft: `${6 + depth * 14}px` };

        if (entry.is_dir) {
          const open = ctx.expanded.has(childPath);
          return (
            <div key={childPath}>
              <div
                className={rowClass}
                style={indent}
                role="button"
                tabIndex={0}
                aria-expanded={open}
                onClick={() => ctx.toggle(childPath)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    ctx.toggle(childPath);
                  }
                }}
              >
                {open
                  ? <ChevronDown size={12} className="shrink-0" />
                  : <ChevronRight size={12} className="shrink-0" />}
                {open
                  ? <FolderOpen size={13} className="shrink-0 text-faint" />
                  : <Folder size={13} className="shrink-0 text-faint" />}
                <span className="truncate">{entry.name}</span>
              </div>
              {open && <DirRows path={childPath} depth={depth + 1} ctx={ctx} />}
            </div>
          );
        }

        const status = ctx.gitMap.get(childPath);
        const icon = isTextFile(entry.name)
          ? <File size={13} className="shrink-0 text-faint" />
          : <FileText size={13} className="shrink-0 text-faint" />;
        return (
          <div
            key={childPath}
            className={rowClass}
            style={indent}
            role="button"
            tabIndex={0}
            onClick={() => ctx.onOpenFile(childPath)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                ctx.onOpenFile(childPath);
              }
            }}
          >
            <span className="w-3 shrink-0" />
            {icon}
            <span className="truncate">{entry.name}</span>
            {status && (
              <Badge tone={badgeTone(status)} size="sm" className="ml-auto shrink-0 font-mono">
                {badgeLetter(status)}
              </Badge>
            )}
          </div>
        );
      })}
    </>
  );
}

export function FileTreePane() {
  const project = useActiveProject();
  const root = project?.rootPath ?? null;
  const toggleFileTree = useUi((s) => s.toggleFileTree);
  const openFilePreview = useUi((s) => s.openFilePreview);
  const setView = useUi((s) => s.setView);
  const queryClient = useQueryClient();
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());
  const [showHidden, setShowHidden] = useState(false);
  const { width, setWidth, commitWidth } = usePanelWidth(
    "mix-agent-file-tree-width", 240, 180, 480,
  );

  const git = useQuery({
    queryKey: ["git_status", root],
    queryFn: () => api.gitStatus(root!),
    enabled: !!root,
    refetchInterval: 15_000,
  });

  const gitMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const entry of git.data?.entries ?? []) map.set(entry.path, entry.status);
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
    void queryClient.invalidateQueries({ queryKey: ["git_status", root] });
  };

  return (
    <>
      <aside
        aria-label={t("ファイルツリー")}
        style={{ width }}
        className="flex shrink-0 flex-col overflow-hidden border-e border-line bg-inset"
      >
      <div className="flex h-9 shrink-0 items-center gap-1.5 border-b border-line px-2.5">
        <FolderTree size={13} className="shrink-0 text-faint" />
        <span className="min-w-0 flex-1 truncate text-xs font-semibold text-fg" title={root ?? undefined}>
          {project ? project.name : t("プロジェクト")}
        </span>
        {git.data?.available && git.data.branch && (
          <Badge tone="neutral" size="sm" className="shrink-0 font-mono">
            {git.data.branch}
          </Badge>
        )}
        <Button
          variant="ghost"
          size="icon-sm"
          title={showHidden ? t("隠しファイルを隠す") : t("隠しファイルを表示")}
          aria-label={showHidden ? t("隠しファイルを隠す") : t("隠しファイルを表示")}
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
        <Button
          variant="ghost"
          size="icon-sm"
          title={t("閉じる")}
          aria-label={t("閉じる")}
          onClick={toggleFileTree}
        >
          <X size={12} />
        </Button>
      </div>

      {root ? (
        <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-1.5">
          <DirRows path="" depth={0} ctx={ctx} />
        </div>
      ) : (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 p-4 text-center">
          <p className="m-0 text-xs text-faint">{t("プロジェクトにフォルダを設定してください")}</p>
          <Button size="sm" onClick={() => setView("projects")}>
            {t("プロジェクト")}
          </Button>
        </div>
      )}

      {root && git.isError && (
        <p className="px-3 py-2 text-2xs text-danger">{t("git を利用できません")}</p>
      )}
      </aside>
      <ResizableHandle
        value={width}
        min={180}
        max={480}
        label={t("ファイルツリーの幅")}
        onChange={setWidth}
        onCommit={commitWidth}
      />
    </>
  );
}
