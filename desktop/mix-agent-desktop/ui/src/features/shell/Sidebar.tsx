import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  Bot, Clock3, Download, Folder, MessageSquare, Monitor, Moon,
  Pencil, Plus, Search, Settings2, Sun, Trash2, X,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { relativeTime, threadGroup, type ThreadGroup } from "@/lib/format";
import {
  SURFACE_MODES, useUi, type Surface, type ThreadFilter, type View,
} from "@/lib/ui";
import type { ConversationSummary } from "@/lib/types";
import { errorMessage } from "@/lib/errors";
import { useApplySettings, PENDING_APPROVALS_POLL_MS } from "@/lib/settings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ResizableHandle } from "./ResizableHandle";
import { useCreateThread } from "./useShellEvents";
import { usePanelWidth } from "./usePanelWidth";

interface NavEntry {
  id: View;
  label: string;
  icon: React.ReactNode;
}

const NAV_GROUPS: { title: string; items: NavEntry[] }[] = [
  {
    title: "ワークスペース",
    items: [
      { id: "chat", label: "チャット", icon: <MessageSquare size={15} /> },
      { id: "projects", label: "プロジェクト", icon: <Folder size={15} /> },
      { id: "agents", label: "エージェント", icon: <Bot size={15} /> },
      { id: "schedules", label: "スケジュール", icon: <Clock3 size={15} /> },
    ],
  },
  {
    title: "システム",
    items: [
      { id: "settings", label: "設定", icon: <Settings2 size={15} /> },
    ],
  },
];

const GROUP_ORDER: ThreadGroup[] = ["today", "yesterday", "week", "older"];
const GROUP_LABEL: Record<ThreadGroup, string> = {
  today: "今日",
  yesterday: "昨日",
  week: "過去7日",
  older: "それより前",
};

const THEME_ICON = {
  light: Sun,
  dark: Moon,
  system: Monitor,
} as const;

const THEME_NEXT: Record<"light" | "dark" | "system", "light" | "dark" | "system"> = {
  light: "dark",
  dark: "system",
  system: "light",
};

export function Sidebar() {
  const view = useUi((s) => s.view);
  const setView = useUi((s) => s.setView);
  const conversationId = useUi((s) => s.conversationId);
  const setConversationId = useUi((s) => s.setConversationId);
  const locale = useUi((s) => s.locale);
  const setLocale = useUi((s) => s.setLocale);
  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);
  const sidebarOpen = useUi((s) => s.sidebarOpen);
  const activeProjectId = useUi((s) => s.activeProjectId);
  const setActiveProjectId = useUi((s) => s.setActiveProjectId);
  const threadFilter = useUi((s) => s.threadFilter);
  const setThreadFilter = useUi((s) => s.setThreadFilter);
  const { width, setWidth, commitWidth } = usePanelWidth(
    "mix-agent-sidebar-width", 264, 210, 440,
  );
  const applySettings = useApplySettings();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");

  const conversations = useQuery({ queryKey: ["conversations"], queryFn: api.listConversations });
  const projects = useQuery({ queryKey: ["projects"], queryFn: api.listProjects });
  const approvals = useQuery({
    queryKey: ["pending_approvals"],
    queryFn: api.listPendingApprovals,
    refetchInterval: PENDING_APPROVALS_POLL_MS,
  });

  const activeProject = projects.data?.find((p) => p.id === activeProjectId) ?? null;

  const createConversation = useCreateThread();

  const rename = useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      api.renameConversation(id, title),
    onSuccess: () => {
      setRenaming(null);
      void queryClient.invalidateQueries({ queryKey: ["conversations"] });
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteConversation(id),
    onSuccess: (_data, id) => {
      void queryClient.invalidateQueries({ queryKey: ["conversations"] });
      if (conversationId === id) setConversationId(null);
    },
  });

  const exportThread = async (conversation: ConversationSummary) => {
    const safe = conversation.title.replace(/[\\/:*?"<>|]/g, "_");
    const path = await saveDialog({
      defaultPath: `${safe}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!path) return;
    const markdown = await api.exportMarkdown(conversation.id, true);
    await api.saveTextFile(path, markdown);
  };

  // Counts per surface feed the filter segment's badges. Computed before the
  // search box filters anything so the counts reflect the full conversation
  // list within the current project filter.
  const filterCounts = useMemo(() => {
    const scoped = (conversations.data ?? []).filter(
      (c) => !activeProjectId || c.project_id === activeProjectId,
    );
    const chat = scoped.filter((c) => SURFACE_MODES.chat.includes(c.mode as never)).length;
    const desktopAgent = scoped.filter(
      (c) => SURFACE_MODES["desktop-agent"].includes(c.mode as never),
    ).length;
    return {
      all: scoped.length,
      chat,
      "desktop-agent": desktopAgent,
    } satisfies Record<ThreadFilter, number>;
  }, [conversations.data, activeProjectId]);

  const threads = useMemo(() => {
    const q = search.trim().toLowerCase();
    const filterSurface: Surface | null = threadFilter === "all" ? null : threadFilter;
    const allowedModes: ReadonlyArray<string> | null =
      filterSurface === null ? null : SURFACE_MODES[filterSurface];
    const list = (conversations.data ?? []).filter(
      (c) =>
        (!activeProjectId || c.project_id === activeProjectId)
        && (allowedModes === null || allowedModes.includes(c.mode))
        && (!q || c.title.toLowerCase().includes(q) || (c.preview ?? "").toLowerCase().includes(q)),
    );
    const groups = new Map<ThreadGroup, ConversationSummary[]>();
    for (const c of list) {
      const group = threadGroup(c.updated_at);
      const bucket = groups.get(group) ?? [];
      bucket.push(c);
      groups.set(group, bucket);
    }
    return groups;
  }, [conversations.data, search, activeProjectId, threadFilter]);

  const filterOptions: Array<{ id: ThreadFilter; label: string; Icon: typeof MessageSquare }> = [
    { id: "all", label: t("すべてのスレッド"), Icon: MessageSquare },
    { id: "chat", label: t("chat"), Icon: MessageSquare },
    { id: "desktop-agent", label: t("desktop-agent"), Icon: Bot },
  ];

  const pendingCount = approvals.data?.length ?? 0;
  const hasThreads = (conversations.data?.length ?? 0) > 0;
  // After the project / surface / search filter. The "nothing matched" message
  // used to be guarded on `conversations.data.length === 0` *inside* the
  // `hasThreads` block, so it could never render.
  const filteredThreadCount = [...threads.values()].reduce((n, items) => n + items.length, 0);
  const ThreadIcon = THEME_ICON[theme];
  const openThread = (c: ConversationSummary) => {
    setConversationId(c.id);
    if (c.project_id) setActiveProjectId(c.project_id);
    setView("chat");
  };

  return (
    <>
      <aside
        data-collapsed={!sidebarOpen}
        aria-label={t("サイドバー")}
        style={{ "--sidebar-w": `${width}px` } as React.CSSProperties}
        className={cn(
          "flex w-(--sidebar-w) shrink-0 flex-col overflow-hidden border-r border-line bg-sidebar",
          "transition-[margin] duration-150 ease-out",
          sidebarOpen ? "mr-0" : "-mr-(--sidebar-w)",
        )}
      >
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-2">
        <Button
          variant="primary"
          size="lg"
          className="w-full justify-start"
          onClick={() => createConversation.mutate()}
          disabled={createConversation.isPending}
        >
          <Plus size={15} />
          {t("新規スレッド")}
        </Button>

        <label className="flex h-8 items-center gap-2 rounded-md border border-line bg-surface px-2.5 transition-colors hover:border-line-strong focus-within:border-brand focus-within:ring-2 focus-within:ring-focus">
          <Search size={13} className="shrink-0 text-faint" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("スレッドを検索")}
            aria-label={t("スレッドを検索")}
            className="min-w-0 flex-1 bg-transparent text-sm text-fg outline-none placeholder:text-faint"
          />
          {search && (
            <button
              type="button"
              onClick={() => setSearch("")}
              aria-label={t("クリア")}
              className="shrink-0 text-faint transition-colors hover:text-fg"
            >
              <X size={11} />
            </button>
          )}
        </label>

        {(filterCounts.all > 0 || filterCounts.chat > 0 || filterCounts["desktop-agent"] > 0) && (
          <div
            role="tablist"
            aria-label={t("サーフェスで絞り込み")}
            className="flex h-7 items-center gap-0.5 rounded-md border border-line bg-surface p-0.5"
          >
            {filterOptions.map((opt) => {
              const selected = threadFilter === opt.id;
              return (
                <button
                  key={opt.id}
                  type="button"
                  role="tab"
                  aria-selected={selected}
                  data-testid={`thread-filter-${opt.id}`}
                  onClick={() => setThreadFilter(opt.id)}
                  className={cn(
                    "flex h-6 min-w-0 flex-1 items-center justify-center gap-1 rounded px-1.5",
                    "text-2xs font-medium transition-colors duration-100",
                    selected
                      ? "bg-bg text-fg shadow-xs"
                      : "text-muted hover:text-fg",
                  )}
                  title={opt.label}
                >
                  <opt.Icon size={11} className="shrink-0" aria-hidden="true" />
                  <span className="truncate">{opt.label}</span>
                  <span
                    className={cn(
                      "shrink-0 rounded-sm px-1 font-mono text-2xs leading-4",
                      selected ? "bg-active text-fg" : "text-faint",
                    )}
                  >
                    {filterCounts[opt.id]}
                  </span>
                </button>
              );
            })}
          </div>
        )}

        {NAV_GROUPS.map((group) => (
          <nav key={group.title} className="flex flex-col gap-0.5">
            <h2 className="px-2 pb-1 text-2xs font-semibold uppercase tracking-wide text-faint">
              {t(group.title)}
            </h2>
            {group.items.map((item) => (
              <button
                type="button"
                key={item.id}
                onClick={() => setView(item.id)}
                aria-current={view === item.id ? "page" : undefined}
                className={cn(
                  "flex h-7 w-full items-center gap-2 rounded-md px-2 text-sm",
                  "text-muted transition-colors duration-100 hover:bg-hover hover:text-fg",
                  "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus",
                  view === item.id && "bg-active font-medium text-fg",
                )}
              >
                <span className="shrink-0 text-faint">{item.icon}</span>
                <span className="truncate">{t(item.label)}</span>
                {item.id === "settings" && pendingCount > 0 && (
                  <Badge tone="warning" size="sm" className="ml-auto">
                    {pendingCount}
                  </Badge>
                )}
              </button>
            ))}
          </nav>
        ))}

        {(projects.data?.length ?? 0) > 0 && (
          <nav className="flex flex-col gap-0.5">
            <h2 className="px-2 pb-1 text-2xs font-semibold uppercase tracking-wide text-faint">
              {t("プロジェクト")}
            </h2>
            {projects.data!.map((p) => (
              <button
                type="button"
                key={p.id}
                title={p.rootPath ?? p.name}
                onClick={() => {
                  setActiveProjectId(activeProjectId === p.id ? null : p.id);
                  setView("chat");
                }}
                className={cn(
                  "flex h-7 w-full items-center gap-2 rounded-md px-2 text-sm",
                  "text-muted transition-colors duration-100 hover:bg-hover hover:text-fg",
                  "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus",
                  activeProjectId === p.id && "bg-active font-medium text-fg",
                )}
              >
                <Folder size={15} className="shrink-0 text-faint" />
                <span className="truncate">{p.name}</span>
              </button>
            ))}
          </nav>
        )}

        {activeProject && (
          <Button
            size="sm"
            variant="default"
            className="mx-1 w-[calc(100%-0.5rem)] justify-start"
            onClick={() => setActiveProjectId(null)}
            title={t("プロジェクトフィルタを解除")}
          >
            <Folder size={13} className="shrink-0 text-faint" />
            <span className="truncate">{activeProject.name}</span>
            <X size={13} className="ml-auto shrink-0 text-faint" />
          </Button>
        )}

        {hasThreads && (
          <div className="flex flex-1 flex-col gap-2">
            {GROUP_ORDER.map((group) => {
              const items = threads.get(group);
              if (!items?.length) return null;
              return (
                <section key={group}>
                  <h2 className="flex items-center gap-1 px-2 pb-1 text-2xs font-semibold uppercase tracking-wide text-faint">
                    {t(GROUP_LABEL[group])}
                    <span className="font-mono font-normal text-faint/70">{items.length}</span>
                  </h2>
                  <ul className="flex flex-col gap-0.5">
                    {items.map((c) => (
                      <li key={c.id}>
                        <div
                          role="button"
                          tabIndex={0}
                          onClick={() => openThread(c)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter" || e.key === " ") {
                              e.preventDefault();
                              openThread(c);
                            }
                          }}
                          aria-current={conversationId === c.id ? "true" : undefined}
                          className={cn(
                            "group relative flex cursor-pointer flex-col gap-0.5 rounded-md py-1.5 pl-2 pr-2",
                            "text-left transition-colors duration-100 hover:bg-hover",
                            "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus",
                            conversationId === c.id && "bg-active",
                          )}
                        >
                          {renaming === c.id ? (
                            <Input
                              className="h-6 px-1.5 text-xs"
                              value={renameDraft}
                              autoFocus
                              onClick={(e) => e.stopPropagation()}
                              onChange={(e) => setRenameDraft(e.target.value)}
                              onKeyDown={(e) => {
                                e.stopPropagation();
                                if (e.key === "Enter" && renameDraft.trim()) {
                                  rename.mutate({ id: c.id, title: renameDraft.trim() });
                                }
                                if (e.key === "Escape") setRenaming(null);
                              }}
                              onBlur={() => {
                                if (renameDraft.trim() && renameDraft !== c.title) {
                                  rename.mutate({ id: c.id, title: renameDraft.trim() });
                                } else {
                                  setRenaming(null);
                                }
                              }}
                            />
                          ) : (
                            <>
                              <span className="flex min-w-0 items-center gap-1.5 pr-14">
                                {c.project_id && (
                                  <span
                                    className="size-1.5 shrink-0 rounded-full bg-brand"
                                    aria-hidden="true"
                                    title={t("プロジェクト紐付けあり")}
                                  />
                                )}
                                <span className="truncate text-sm font-medium text-fg">
                                  {c.title}
                                </span>
                              </span>
                              {c.preview && (
                                <span className="truncate text-xs text-muted">{c.preview}</span>
                              )}
                              <span className="flex items-center gap-1 text-2xs text-faint">
                                <span>{relativeTime(c.updated_at)}</span>
                                <span className="text-line">·</span>
                                <span className="font-mono uppercase">{c.mode}</span>
                                {c.message_count > 0 && (
                                  <>
                                    <span className="text-line">·</span>
                                    <span className="font-mono">{c.message_count}</span>
                                  </>
                                )}
                              </span>

                              {/* Revealed on hover/focus only, so the list reads as
                                  text rather than as a wall of icons. */}
                              <span
                                className={cn(
                                  "absolute right-1 top-1.5 flex gap-0.5",
                                  "opacity-0 transition-opacity duration-100",
                                  "group-hover:opacity-100 group-focus-within:opacity-100",
                                )}
                              >
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  title={t("名前を変更")}
                                  aria-label={t("名前を変更")}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    setRenaming(c.id);
                                    setRenameDraft(c.title);
                                  }}
                                >
                                  <Pencil size={12} />
                                </Button>
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  title={t("エクスポート")}
                                  aria-label={t("エクスポート")}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    void exportThread(c);
                                  }}
                                >
                                  <Download size={12} />
                                </Button>
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  className="text-danger hover:text-danger"
                                  title={t("削除")}
                                  aria-label={t("削除")}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    if (window.confirm(t("このスレッドを削除しますか？"))) {
                                      remove.mutate(c.id);
                                    }
                                  }}
                                >
                                  <Trash2 size={12} />
                                </Button>
                              </span>
                            </>
                          )}
                        </div>
                      </li>
                    ))}
                  </ul>
                </section>
              );
            })}
            {filteredThreadCount === 0 && (
              <p className="px-2 text-xs text-faint">{t("一致するスレッドがありません")}</p>
            )}
          </div>
        )}

        {conversations.isError && (
          <div
            role="alert"
            className="mt-2 rounded-lg border border-danger/40 bg-danger/10 px-3 py-3 text-xs text-danger"
          >
            {errorMessage(conversations.error) || t("エラー")}
          </div>
        )}
        {!conversations.isError && !hasThreads && !conversations.isPending && (
          <div className="mt-2 flex flex-col items-start gap-1 rounded-lg border border-dashed border-line px-3 py-3 text-start">
            <span className="text-xs font-medium text-muted">
              {t("スレッドがありません")}
            </span>
            <span className="text-2xs text-faint leading-4">
              {t("新しい会話を始めましょう")}
            </span>
          </div>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-1.5 border-t border-line px-2 py-1.5">
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            const next = locale === "ja" ? "en" : "ja";
            setLocale(next);
            // Persisted immediately: a local-only switch was overwritten by
            // the stored setting on the next launch.
            void applySettings({ language: next });
          }}
          title={t("言語")}
        >
          {locale === "ja" ? "EN" : "JA"}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            const next = THEME_NEXT[theme];
            setTheme(next);
            void applySettings({ theme: next });
          }}
          title={t("テーマ")}
          aria-label={t("テーマ")}
        >
          <ThreadIcon size={14} />
        </Button>
        <span className="flex-1" />
        <span className="text-2xs text-faint">MIX-agent</span>
      </div>
      </aside>
      {sidebarOpen && (
        <ResizableHandle
          value={width}
          min={210}
          max={440}
          label={t("サイドバーの幅")}
          onChange={setWidth}
          onCommit={commitWidth}
        />
      )}
    </>
  );
}
