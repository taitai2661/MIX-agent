import { useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
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
import {
  CONVERSATIONS_QUERY_KEY, useConversations, usePendingApprovals, useProjects,
} from "@/lib/queries";
import { useApplySettings } from "@/lib/settings";
import { Alert, Skeleton } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { Input, SearchField } from "@/components/ui/input";
import { NavRow, SectionLabel } from "@/components/layout";
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
  const { confirm, dialog } = useConfirm();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");

  const conversations = useConversations();
  const projects = useProjects();
  const approvals = usePendingApprovals();

  const activeProject = projects.data?.find((p) => p.id === activeProjectId) ?? null;

  const createConversation = useCreateThread();

  const rename = useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      api.renameConversation(id, title),
    onSuccess: () => {
      setRenaming(null);
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteConversation(id),
    onSuccess: (_data, id) => {
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
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

        <SearchField
          icon={<Search size={13} />}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("スレッドを検索")}
          aria-label={t("スレッドを検索")}
          {...(search ? { onClear: () => setSearch(""), clearLabel: t("クリア") } : {})}
        />

        {(filterCounts.all > 0 || filterCounts.chat > 0 || filterCounts["desktop-agent"] > 0) && (
          <Segmented aria-label={t("サーフェスで絞り込み")}>
            {filterOptions.map((opt) => (
              <SegmentedOption
                key={opt.id}
                value={opt.id}
                label={opt.label}
                selected={threadFilter === opt.id}
                onSelect={setThreadFilter}
                data-testid={`thread-filter-${opt.id}`}
                trailing={filterCounts[opt.id]}
              >
                <opt.Icon size={11} className="shrink-0" aria-hidden="true" />
                {opt.label}
              </SegmentedOption>
            ))}
          </Segmented>
        )}

        {NAV_GROUPS.map((group) => (
          <nav key={group.title} className="flex flex-col gap-0.5">
            <SectionLabel>{t(group.title)}</SectionLabel>
            {group.items.map((item) => (
              <NavRow
                key={item.id}
                icon={item.icon}
                selected={view === item.id}
                onClick={() => setView(item.id)}
                trailing={
                  item.id === "settings" && pendingCount > 0
                    ? <Badge tone="warning" size="sm">{pendingCount}</Badge>
                    : undefined
                }
              >
                {t(item.label)}
              </NavRow>
            ))}
          </nav>
        ))}

        {(projects.data?.length ?? 0) > 0 && (
          <nav className="flex flex-col gap-0.5">
            <SectionLabel>{t("プロジェクト")}</SectionLabel>
            {projects.data!.map((p) => (
              <NavRow
                key={p.id}
                icon={<Folder size={15} />}
                title={p.rootPath ?? p.name}
                selected={activeProjectId === p.id}
                onClick={() => {
                  setActiveProjectId(activeProjectId === p.id ? null : p.id);
                  setView("chat");
                }}
              >
                {p.name}
              </NavRow>
            ))}
          </nav>
        )}

        {activeProject && (
          <Button
            size="sm"
            variant="default"
            className="mx-1 w-[calc(100%_-_0.5rem)] justify-start"
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
                  <SectionLabel trailing={
                    <span className="font-mono font-normal text-faint/70">{items.length}</span>
                  }>
                    {t(GROUP_LABEL[group])}
                  </SectionLabel>
                  <ul className="flex flex-col gap-0.5">
                    {items.map((c) => (
                      <li key={c.id}>
                        {/* A real `<button>`, not a `div role="button"`. The row also
                            holds rename / export / delete buttons, and nesting those
                            inside another button is invalid — so the title is the
                            button and it stretches over the row with a pseudo-element,
                            which keeps the whole row clickable without the invalid
                            nesting. */}
                        <div
                          className={cn(
                            "group relative flex flex-col gap-0.5 rounded-md py-1.5 pe-2 ps-2",
                            "transition-colors duration-100 hover:bg-hover",
                            conversationId === c.id && "bg-active",
                          )}
                        >
                          {renaming === c.id ? (
                            <Input
                              className="h-6 px-1.5 text-xs"
                              value={renameDraft}
                              autoFocus
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
                              <button
                                type="button"
                                onClick={() => openThread(c)}
                                aria-current={conversationId === c.id ? "true" : undefined}
                                className="flex min-w-0 items-center gap-1.5 text-start after:absolute after:inset-0 after:content-['']"
                              >
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
                              </button>
                              {c.preview && (
                                <span className="pointer-events-none truncate text-xs text-muted">
                                  {c.preview}
                                </span>
                              )}
                              <span className="pointer-events-none flex items-center gap-1 text-xs text-faint">
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
                                  text rather than as a wall of icons. `relative` keeps
                                  them above the title's stretched pseudo-element. */}
                              <span
                                className={cn(
                                  "absolute end-1 top-1.5 flex gap-0.5",
                                  "opacity-0 transition-opacity duration-100",
                                  "group-hover:opacity-100 group-focus-within:opacity-100",
                                )}
                              >
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  title={t("名前を変更")}
                                  aria-label={t("名前を変更")}
                                  onClick={() => {
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
                                  onClick={() => void exportThread(c)}
                                >
                                  <Download size={12} />
                                </Button>
                                <Button
                                  variant="ghost-danger"
                                  size="icon-sm"
                                  title={t("削除")}
                                  aria-label={t("削除")}
                                  onClick={async () => {
                                    if (await confirm({
                                      title: t("スレッドを削除しますか？"),
                                      description: c.title,
                                      confirmLabel: t("削除"),
                                      tone: "danger",
                                    })) {
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
          <Alert tone="danger" className="mt-2">
            {errorMessage(conversations.error) || t("エラー")}
          </Alert>
        )}
        {!conversations.isError && !hasThreads && !conversations.isPending && (
          <div className="mt-2 flex flex-col items-start gap-1 rounded-lg border border-dashed border-line px-3 py-3 text-start">
            <span className="text-xs font-medium text-muted">
              {t("スレッドがありません")}
            </span>
            <span className="text-xs leading-4 text-faint">
              {t("新しい会話を始めましょう")}
            </span>
          </div>
        )}
        {conversations.isPending && (
          <div aria-hidden className="mt-2 flex flex-col gap-1.5">
            {[0, 1, 2].map((i) => (
              <div key={i} className="rounded-md border border-line px-2 py-2">
                <Skeleton className="h-3 w-2/3" />
                <Skeleton className="mt-1.5 h-2.5 w-1/3" />
              </div>
            ))}
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
        <span className="text-xs text-faint">MIX-agent</span>
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
      {dialog}
    </>
  );
}
