import {
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AtSign,
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
  Search,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { useActiveProject, gitStatusKey, useGitStatus } from "@/lib/queries";
import { usePromptInsert } from "@/lib/promptInsert";
import { getString, setString } from "@/lib/storage";
import { PanelHeader } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SearchField } from "@/components/ui/input";
import { GitStatusBadge } from "@/components/ui/status";
import { EmptyState, ErrorAlert, ListSkeleton } from "@/components/shared";
import { gitStatusLabel, gitStatusLetter, gitStatusTone } from "@/lib/git";
import { panelStyle } from "@/features/shell/usePanelCap";

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
  onAddToChat: (path: string) => void;
}

const DirRows = memo(function DirRows({
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
                data-tree-row
                data-tree-path={childPath}
                data-tree-depth={depth}
                data-tree-dir="true"
                data-tree-open={open ? "true" : "false"}
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
          <div
            key={childPath}
            className="group/row relative flex items-center"
            style={indent}
          >
            <button
              type="button"
              data-tree-row
              data-tree-path={childPath}
              data-tree-depth={depth}
              className={rowClass}
              onClick={() => ctx.onOpenFile(childPath)}
              title={
                status
                  ? `${entry.name} — ${gitStatusLabel(status)}`
                  : entry.name
              }
            >
              <span className="w-3 shrink-0" />
              {icon}
              <span className="min-w-0 flex-1 truncate">{entry.name}</span>
              {status && (
                <GitStatusBadge
                  letter={gitStatusLetter(status)}
                  label={gitStatusLabel(status)}
                  tone={gitStatusTone(status)}
                />
              )}
            </button>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                ctx.onAddToChat(childPath);
              }}
              title={tf("チャットに入力 (@{path})", { path: childPath })}
              aria-label={t("チャットに入力")}
              className="absolute right-1 hidden h-5 w-5 items-center justify-center rounded bg-surface/80 text-muted shadow-xs hover:bg-surface hover:text-brand group-hover/row:flex"
            >
              <AtSign size={11} />
            </button>
          </div>
        );
      })}
    </>
  );
});

/** Fixed width, like the sidebar: dragging the tree's edge used to reflow the
 * transcript on every pointermove. */
const FILE_TREE_WIDTH = 240;

export function FileTreePane({ cap = null }: { cap?: number | null }) {
  const project = useActiveProject();
  const root = project?.rootPath ?? null;
  const toggleFileTree = useUi((s) => s.toggleFileTree);
  const openFilePreview = useUi((s) => s.openFilePreview);
  const setView = useUi((s) => s.setView);
  const queryClient = useQueryClient();
  const treeRef = useRef<HTMLDivElement | null>(null);
  const expandedKey = `mix:file-tree-expanded:${root ?? ""}`;
  // Expansion used to be component-local, so closing the tree (which unmounts
  // this pane) collapsed every folder and destroyed the user's orientation.
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [showHidden, setShowHidden] = useState(false);
  const [filter, setFilter] = useState("");
  const deferredFilter = useDeferredValue(filter.trim());
  // The backend already walks the tree with the project's .gitignore rules;
  // reusing it keeps search results identical to what the agent can read.
  const search = useQuery({
    queryKey: ["workspace_files", root, deferredFilter],
    queryFn: () => api.findWorkspaceFiles(root!, deferredFilter, 80),
    enabled: !!root && deferredFilter.length > 0,
    staleTime: 10_000,
  });
  // Only persist after a user toggle, otherwise the mount-time effect would
  // write the initial empty set over the paths restored from storage.
  const expandedDirty = useRef(false);

  useEffect(() => {
    if (!root) return;
    expandedDirty.current = false;
    try {
      const raw = getString(expandedKey);
      const parsed: unknown = raw ? JSON.parse(raw) : [];
      setExpanded(
        new Set(
          Array.isArray(parsed)
            ? parsed.filter((p): p is string => typeof p === "string")
            : [],
        ),
      );
    } catch {
      setExpanded(new Set());
    }
  }, [expandedKey, root]);

  // Keyboard travel over the visible rows (the sidebar already does this).
  // ArrowRight expands a folder, ArrowLeft collapses it or jumps to its parent.
  const onTreeKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const target = (event.target as HTMLElement).closest<HTMLElement>(
      "[data-tree-row]",
    );
    if (!target) return;
    const rows = Array.from(
      treeRef.current?.querySelectorAll<HTMLElement>("[data-tree-row]") ?? [],
    );
    const index = rows.indexOf(target);
    if (index === -1) return;
    const depth = Number(target.dataset.treeDepth ?? "0");

    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      rows[
        Math.min(
          rows.length - 1,
          Math.max(0, index + (event.key === "ArrowDown" ? 1 : -1)),
        )
      ]?.focus();
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      rows[event.key === "Home" ? 0 : rows.length - 1]?.focus();
      return;
    }
    if (event.key === "ArrowRight") {
      event.preventDefault();
      if (
        target.dataset.treeDir === "true" &&
        target.dataset.treeOpen !== "true"
      ) {
        target.click();
      } else {
        rows[index + 1]?.focus();
      }
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (
        target.dataset.treeDir === "true" &&
        target.dataset.treeOpen === "true"
      ) {
        target.click();
        return;
      }
      for (let i = index - 1; i >= 0; i -= 1) {
        if (Number(rows[i].dataset.treeDepth ?? "0") < depth) {
          rows[i].focus();
          break;
        }
      }
    }
  };

  const git = useGitStatus(root);

  const gitMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const entry of git.data?.entries ?? [])
      map.set(entry.path, entry.status);
    return map;
  }, [git.data]);

  const toggle = useCallback((path: string) => {
    expandedDirty.current = true;
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  useEffect(() => {
    if (!root || !expandedDirty.current) return;
    setString(expandedKey, JSON.stringify([...expanded]));
  }, [expanded, expandedKey, root]);

  const insertFileReference = usePromptInsert((s) => s.insertFileReference);

  // Stable identity so memoised `DirRows` subtrees survive parent re-renders
  // (git poll, panel resize) instead of remounting every 15s.
  const ctx: TreeContext = useMemo(
    () => ({
      root: root ?? "",
      expanded,
      toggle,
      gitMap,
      showHidden,
      onOpenFile: openFilePreview,
      onAddToChat: (path: string) => insertFileReference(path),
    }),
    [
      root,
      expanded,
      toggle,
      gitMap,
      showHidden,
      openFilePreview,
      insertFileReference,
    ],
  );

  const refresh = () => {
    if (!root) return;
    void queryClient.invalidateQueries({ queryKey: ["dir", root] });
    void queryClient.invalidateQueries({ queryKey: gitStatusKey(root ?? "") });
  };

  return (
    <aside
      aria-label={t("ファイルツリー")}
      data-side-panel=""
      style={panelStyle(FILE_TREE_WIDTH, cap)}
      className="flex shrink-0 flex-col overflow-hidden border-e border-line bg-sidebar"
    >
      <PanelHeader
        icon={<FolderTree size={14} />}
        title={project ? project.name : t("プロジェクト")}
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

      {root && (
        <div className="shrink-0 px-2 pb-1.5">
          <SearchField
            icon={<Search size={12} />}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={t("ファイルを検索")}
            aria-label={t("ファイルを検索")}
            {...(filter
              ? { onClear: () => setFilter(""), clearLabel: t("クリア") }
              : {})}
          />
        </div>
      )}

      {root ? (
        deferredFilter ? (
          // Keyboard travel is delegated from the container to its native
          // buttons, the same pattern the thread sidebar uses.
          // eslint-disable-next-line jsx-a11y/no-static-element-interactions
          <div
            ref={treeRef}
            onKeyDown={onTreeKeyDown}
            aria-label={t("検索結果")}
            className="stable-scroll flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-1.5"
          >
            {search.isPending ? (
              <ListSkeleton rows={3} />
            ) : search.isError ? (
              <ErrorAlert error={search.error} />
            ) : (search.data ?? []).length === 0 ? (
              <EmptyState compact title={t("一致するファイルがありません")} />
            ) : (
              (search.data ?? []).map((path) => {
                const at = path.lastIndexOf("/");
                return (
                  <button
                    key={path}
                    type="button"
                    data-tree-row
                    data-tree-path={path}
                    data-tree-depth={0}
                    onClick={() => openFilePreview(path)}
                    title={path}
                    className={rowClass}
                  >
                    <File size={12} className="shrink-0 text-faint" />
                    <span className="min-w-0 flex-1 truncate">
                      {at >= 0 ? (
                        <>
                          <span className="text-faint">
                            {path.slice(0, at + 1)}
                          </span>
                          {path.slice(at + 1)}
                        </>
                      ) : (
                        path
                      )}
                    </span>
                  </button>
                );
              })
            )}
          </div>
        ) : (
          // eslint-disable-next-line jsx-a11y/no-static-element-interactions
          <div
            ref={treeRef}
            onKeyDown={onTreeKeyDown}
            aria-label={t("ファイル")}
            className="stable-scroll flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-1.5"
          >
            <DirRows path="" depth={0} ctx={ctx} />
          </div>
        )
      ) : (
        <EmptyState
          compact
          className="flex flex-1 flex-col items-center justify-center"
          title={t("プロジェクトにフォルダを設定してください")}
          action={
            <Button size="sm" onClick={() => setView("projects")}>
              {t("プロジェクト")}
            </Button>
          }
        />
      )}

      {root && git.isError && (
        <div className="p-2">
          <ErrorAlert error={t("git を利用できません")} />
        </div>
      )}
    </aside>
  );
}
