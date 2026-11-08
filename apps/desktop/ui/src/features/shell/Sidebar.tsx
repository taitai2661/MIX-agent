import { useDeferredValue, useEffect, useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Bot,
  Check,
  ChevronDown,
  Clock3,
  Folder,
  Languages,
  Monitor,
  Moon,
  Plus,
  Search,
  Settings2,
  Sun,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { threadGroup } from "@/lib/format";
import { useNow } from "@/lib/clock";
import { loadPinnedThreadIds, togglePinnedThreadId } from "@/lib/pinnedThreads";
import { useUi, surfaceForMode, type View } from "@/lib/ui";
import type { ConversationSummary } from "@/lib/types";
import {
  CONVERSATIONS_QUERY_KEY,
  useConversationPages,
  useConversationSearch,
  useProjects,
} from "@/lib/queries";
import { useApplySettings, useSettingsForm } from "@/lib/settings";
import { useRunEvents } from "@/lib/runStore";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useConfirm } from "@/components/ui/dialog";
import { SearchField } from "@/components/ui/input";
import { NavRow, SectionLabel } from "@/components/layout";
import { ErrorBox } from "@/components/shared";
import { useNewThread } from "./useShellEvents";
import { panelStyle } from "./usePanelCap";
import { ResizableHandle } from "./ResizableHandle";
import { usePanelWidth } from "./usePanelWidth";
import { ThreadList } from "./ThreadList";
import type { ExtendedThreadGroup } from "./threadGroups";

/**
 * Secondary destinations, grouped by what they configure rather than by how
 * the backend stores them. Projects are a workspace; saved agent profiles and
 * their schedules are their own concept. Keeping the old single group made the
 * list read as four equal-level features.
 */
const NAV_GROUPS: {
  title: string;
  items: { id: View; label: string; icon: React.ReactNode }[];
}[] = [
  {
    title: "ワークスペース",
    items: [
      { id: "projects", label: "プロジェクト", icon: <Folder size={15} /> },
    ],
  },
  {
    title: "エージェント",
    items: [
      {
        // Not just "エージェント": that word is the workspace and the run mode.
        // This page is the list of saved agent profiles.
        id: "agents",
        label: "エージェント設定",
        icon: <Bot size={15} />,
      },
      { id: "schedules", label: "スケジュール", icon: <Clock3 size={15} /> },
    ],
  },
];

const THEME_ICON = {
  light: Sun,
  dark: Moon,
  system: Monitor,
} as const;

const THEMES = [
  { value: "light", label: "ライト" },
  { value: "dark", label: "ダーク" },
  { value: "system", label: "システム" },
] as const;

const LANGUAGES = [
  { value: "ja", label: "日本語" },
  { value: "en", label: "English" },
] as const;

/** Codex-like default width. The user can drag the right edge to resize
 * (210–440px); the width is remembered across launches. */
const SIDEBAR_WIDTH = 264;
const SIDEBAR_MIN_WIDTH = 210;
const SIDEBAR_MAX_WIDTH = 440;
const SIDEBAR_WIDTH_KEY = "mix-agent-sidebar-width";

export function Sidebar({ cap = null }: { cap?: number | null }) {
  const view = useUi((s) => s.view);
  const setView = useUi((s) => s.setView);
  const conversationId = useUi((s) => s.conversationId);
  const setConversationId = useUi((s) => s.setConversationId);
  const locale = useUi((s) => s.locale);
  const setLocale = useUi((s) => s.setLocale);
  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);
  const sidebarOpen = useUi((s) => s.sidebarOpen);
  const surface = useUi((s) => s.surface);
  const setSurface = useUi((s) => s.setSurface);
  const activeProjectId = useUi((s) => s.activeProjectId);
  const applySettings = useApplySettings();
  const { dirty: settingsDirty } = useSettingsForm();
  const queryClient = useQueryClient();
  const { confirm, dialog } = useConfirm();
  const now = useNow();
  const {
    width: sidebarWidth,
    setWidth: setSidebarWidth,
    commitWidth: commitSidebarWidth,
  } = usePanelWidth(
    SIDEBAR_WIDTH_KEY,
    SIDEBAR_WIDTH,
    SIDEBAR_MIN_WIDTH,
    SIDEBAR_MAX_WIDTH,
  );
  const [search, setSearch] = useState("");
  // Defer the filter so typing stays at 60fps with hundreds of threads.
  const deferredSearch = useDeferredValue(search);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [exportError, setExportError] = useState<string | null>(null);
  const [pinnedIds, setPinnedIds] = useState<string[]>(() =>
    loadPinnedThreadIds(),
  );
  const pinnedSet = useMemo(() => new Set(pinnedIds), [pinnedIds]);

  const togglePin = (id: string) => {
    const next = togglePinnedThreadId(id);
    setPinnedIds(next);
  };

  const isAgent = surface === "desktop-agent";
  const otherSurface = isAgent ? "chat" : "desktop-agent";
  const otherSurfaceLabel = isAgent ? t("チャット") : t("コーディング");
  const projects = useProjects();
  const activeProject =
    projects.data?.find((p) => p.id === activeProjectId) ?? null;
  const conversations = useConversationPages();
  const conversationRows = useMemo(
    () => conversations.data?.pages.flatMap((page) => page.items) ?? [],
    [conversations.data],
  );
  // The sidebar used to filter only the pages already fetched, which read as
  // "history lost" once a thread fell past the first page. Search now asks the
  // server over the whole history; the local rows still render immediately so
  // typing never waits on the round trip.
  const serverSearch = useConversationSearch(deferredSearch);
  const newThread = useNewThread();
  // A run keeps streaming after the user opens another thread. Without this
  // marker the work simply vanished from the UI: the transcript stopped
  // receiving events and nothing else in the shell said a task was still live.
  //
  // Subscribe to the derived *set* instead of `byConversation`: the latter is
  // re-created on every 60 ms flush, which re-rendered this whole list 16×/s
  // while tokens streamed. `busyConversationIds` only changes on a status
  // transition.
  const busyConversationIds = useRunEvents((s) => s.busyConversationIds);
  const runningConversationIds = useMemo(
    () => new Set(busyConversationIds),
    [busyConversationIds],
  );

  const rename = useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      api.renameConversation(id, title),
    onSuccess: () => {
      setRenaming(null);
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    },
  });

  // Enter commits via blur (single path): the input's Enter handler blurs and
  // the blur handler commits, so the two cannot double-fire the mutation.
  const commitRename = (id: string, currentTitle: string) => {
    if (rename.isPending) return;
    const title = renameDraft.trim();
    if (title && title !== currentTitle) {
      rename.reset();
      rename.mutate({ id, title });
    } else {
      setRenaming(null);
    }
  };

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteConversation(id),
    onSuccess: (_data, id) => {
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
      if (conversationId === id) setConversationId(null);
    },
  });

  const exportThread = async (conversation: ConversationSummary) => {
    setExportError(null);
    try {
      const safe = conversation.title.replace(/[\\/:*?"<>|]/g, "_");
      const path = await api.pickSaveFile({
        defaultName: `${safe}.md`,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (!path) return;
      const markdown = await api.exportMarkdown(conversation.id, true);
      await api.saveTextFile(path, markdown);
    } catch (error) {
      // A failed write (full disk, revoked permission, missing folder) used to
      // reject as an unhandled promise and leave the user with no clue why the
      // file was not there.
      setExportError(errorMessage(error));
    }
  };

  const removeThread = async (conversation: ConversationSummary) => {
    const ok = await confirm({
      title: t("スレッドを削除しますか？"),
      description: conversation.title,
      confirmLabel: t("削除"),
      tone: "danger",
    });
    if (ok) {
      remove.reset();
      remove.mutate(conversation.id);
    }
  };

  const threads = useMemo(() => {
    const q = deferredSearch.trim().toLowerCase();
    // Loaded rows first (they carry the preview the server search omits), then
    // server hits so a thread beyond the first page still appears.
    const source = q
      ? [...conversationRows, ...(serverSearch.data ?? [])]
      : conversationRows;
    const seen = new Set<string>();
    const list = source.filter((c) => {
      // Natural separation by active surface:
      if (surfaceForMode(c.mode) !== surface) {
        return false;
      }
      if (seen.has(c.id)) return false;
      seen.add(c.id);
      return (
        !q ||
        c.title.toLowerCase().includes(q) ||
        (c.preview ?? "").toLowerCase().includes(q)
      );
    });
    const groups = new Map<ExtendedThreadGroup, ConversationSummary[]>();
    for (const c of list) {
      if (pinnedSet.has(c.id)) {
        const bucket = groups.get("pinned") ?? [];
        bucket.push(c);
        groups.set("pinned", bucket);
      } else {
        const group = threadGroup(c.updated_at);
        const bucket = groups.get(group) ?? [];
        bucket.push(c);
        groups.set(group, bucket);
      }
    }
    return groups;
  }, [conversationRows, deferredSearch, pinnedSet, serverSearch.data, surface]);

  const hasThreads = conversationRows.some(
    (c) => surfaceForMode(c.mode) === surface,
  );
  const searching = deferredSearch.trim().length > 0;
  const {
    hasNextPage: hasMoreThreads,
    isFetchingNextPage,
    isPending: conversationsPending,
    fetchNextPage,
  } = conversations;
  // While searching, the server may know about threads that the first page
  // does not, so render the list branch even when no local row matches yet.
  const showThreadList = hasThreads || hasMoreThreads || searching;
  const loadMore = () => {
    if (hasMoreThreads && !isFetchingNextPage) {
      void fetchNextPage();
    }
  };

  // Opening a surface whose newest page happens to contain only the *other*
  // surface must not show an empty history: fetch until this surface has at
  // least one row (or the history really is exhausted). A client-side search
  // disables the automatic walk so typing cannot pull every page at once.
  useEffect(() => {
    if (conversationsPending || deferredSearch.trim()) return;
    if (conversationRows.some((c) => surfaceForMode(c.mode) === surface))
      return;
    if (hasMoreThreads && !isFetchingNextPage) {
      void fetchNextPage();
    }
  }, [
    conversationRows,
    conversationsPending,
    deferredSearch,
    fetchNextPage,
    hasMoreThreads,
    isFetchingNextPage,
    surface,
  ]);
  const filteredThreadCount = [...threads.values()].reduce(
    (n, items) => n + items.length,
    0,
  );
  const ThemeIcon = THEME_ICON[theme];
  const openThread = (c: ConversationSummary) => {
    setConversationId(c.id);
    setView("chat");
  };

  // Codex-like: instant show/hide with no width animation. The old
  // negative-margin transition animated the centre column's width for 150ms
  // on every toggle, and the panel-cap observer re-measured mid-animation so
  // the panes visibly jumped. Unmounted when closed: nothing to measure.
  if (!sidebarOpen) {
    return null;
  }

  return (
    <>
      <aside
        aria-label={t("サイドバー")}
        data-side-panel="sidebar"
        // The remembered width; `cap` only clamps it on very narrow windows.
        style={panelStyle(sidebarWidth, cap)}
        className="flex shrink-0 flex-col overflow-hidden border-r border-line bg-sidebar"
      >
        {/* Primary action + search, pinned so they are always reachable. */}
        <div className="flex shrink-0 flex-col gap-2 p-3 pb-1.5">
          {isAgent && (
            <div className="flex items-center justify-between rounded-lg border border-line bg-surface/60 px-2.5 py-1.5 text-xs shadow-xs">
              <div className="flex min-w-0 items-center gap-2">
                <Folder size={14} className="shrink-0 text-brand" />
                <span
                  className="truncate font-medium text-fg"
                  title={
                    activeProject?.rootPath ?? activeProject?.name ?? undefined
                  }
                >
                  {activeProject?.name ?? t("プロジェクト未選択")}
                </span>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setView("projects")}
                className="h-6 shrink-0 px-1.5 text-2xs text-muted hover:text-fg"
                title={t("プロジェクトを変更")}
              >
                {t("変更")}
              </Button>
            </div>
          )}
          <Button
            variant="primary"
            size="lg"
            className="h-10 w-full justify-start gap-2"
            onClick={() => newThread()}
          >
            {isAgent ? <Bot size={15} /> : <Plus size={15} />}
            {isAgent ? t("新規タスク") : t("新規チャット")}
          </Button>
          <SearchField
            icon={<Search size={13} />}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={
              isAgent
                ? t("コーディングの履歴を検索")
                : t("チャットの履歴を検索")
            }
            aria-label={
              isAgent
                ? t("コーディングの履歴を検索（全履歴は ⌘K）")
                : t("チャットの履歴を検索（全履歴は ⌘K）")
            }
            {...(search
              ? { onClear: () => setSearch(""), clearLabel: t("クリア") }
              : {})}
          />
          {/* The two workspaces keep separate histories, but the sidebar used
              to give no sign that the other one existed: switching made the
              list look empty. Name the current scope and offer the other side. */}
          <div className="flex items-center justify-between gap-2 px-1 text-2xs text-faint">
            <span className="truncate">
              {isAgent ? t("コーディングの履歴") : t("チャットの履歴")}
            </span>
            <button
              type="button"
              onClick={() => setSurface(otherSurface)}
              className="shrink-0 rounded px-1 py-0.5 text-brand transition-colors hover:bg-brand-soft hover:text-brand"
              title={tf("{label}の履歴を見る", { label: otherSurfaceLabel })}
            >
              {otherSurfaceLabel} →
            </button>
          </div>
          {(exportError || rename.isError || remove.isError) && (
            <ErrorBox error={exportError ?? rename.error ?? remove.error} />
          )}
        </div>

        <ThreadList
          groups={threads}
          selectedId={conversationId}
          pinned={pinnedSet}
          running={runningConversationIds}
          renamingId={renaming}
          renameDraft={renameDraft}
          onRenameDraftChange={setRenameDraft}
          onCancelRename={() => setRenaming(null)}
          onStartRename={(thread) => {
            setRenaming(thread.id);
            setRenameDraft(thread.title);
          }}
          onCommitRename={commitRename}
          onOpen={openThread}
          onTogglePin={togglePin}
          onExport={(thread) => void exportThread(thread)}
          onRemove={(thread) => void removeThread(thread)}
          removePending={remove.isPending}
          now={now}
          isAgent={isAgent}
          searching={searching}
          serverSearchPending={serverSearch.isPending}
          filteredCount={filteredThreadCount}
          showThreadList={showThreadList}
          hasMore={hasMoreThreads}
          loadingMore={isFetchingNextPage}
          onLoadMore={loadMore}
          onClearSearch={() => setSearch("")}
          conversationsError={
            conversations.isError ? conversations.error : null
          }
          conversationsPending={conversations.isPending}
          otherSurfaceLabel={otherSurfaceLabel}
          onSwitchSurface={() => setSurface(otherSurface)}
        />
        {/* Secondary destinations, below the history so threads stay primary. */}
        <nav className="flex shrink-0 flex-col gap-2 border-t border-line p-3 pt-2.5">
          {/* Shown on both workspaces. They were agent-only, yet ⌘K opened them
            from Chat too — where the sidebar then hid their only navigation and
            the page had no way back. */}
          {NAV_GROUPS.map((group) => (
            <div key={group.title} className="flex flex-col gap-0.5">
              <SectionLabel>{t(group.title)}</SectionLabel>
              {group.items.map((item) => (
                <NavRow
                  key={item.id}
                  icon={item.icon}
                  selected={view === item.id}
                  onClick={() => setView(item.id)}
                >
                  {t(item.label)}
                </NavRow>
              ))}
            </div>
          ))}
          {/* Settings is app-wide, not part of either workspace group. */}
          <div className="border-t border-line pt-1.5">
            <NavRow
              icon={<Settings2 size={15} />}
              selected={view === "settings"}
              onClick={() => setView("settings")}
              trailing={
                settingsDirty ? (
                  <span
                    role="status"
                    aria-label={t("未保存の変更")}
                    title={t("未保存の変更")}
                    className="size-1.5 rounded-full bg-warning"
                  />
                ) : undefined
              }
            >
              {t("設定")}
            </NavRow>
          </div>
        </nav>

        <div className="flex shrink-0 items-center gap-1 border-t border-line px-2.5 py-2">
          <span className="truncate px-1 text-xs font-semibold text-faint">
            MIX-agent
          </span>
          <span className="flex-1" />
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                size="sm"
                variant="ghost"
                className="gap-1 px-2"
                title={t("表示設定")}
                aria-label={t("表示設定")}
              >
                <ThemeIcon size={12} aria-hidden="true" />
                <span className="font-mono text-2xs">
                  {locale === "ja" ? "JA" : "EN"}
                </span>
                <ChevronDown size={10} aria-hidden="true" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-48">
              <DropdownMenuLabel>{t("テーマ")}</DropdownMenuLabel>
              {THEMES.map(({ value, label }) => {
                const Icon = THEME_ICON[value];
                return (
                  <DropdownMenuItem
                    key={value}
                    onSelect={() => {
                      setTheme(value);
                      void applySettings({ theme: value });
                    }}
                  >
                    <Icon size={13} className="text-muted" aria-hidden="true" />
                    <span className="flex-1">{t(label)}</span>
                    {theme === value && (
                      <Check
                        size={13}
                        className="text-brand"
                        aria-hidden="true"
                      />
                    )}
                  </DropdownMenuItem>
                );
              })}
              <DropdownMenuSeparator />
              <DropdownMenuLabel>{t("言語")}</DropdownMenuLabel>
              {LANGUAGES.map(({ value, label }) => (
                <DropdownMenuItem
                  key={value}
                  onSelect={() => {
                    setLocale(value);
                    // Persisted immediately: a local-only switch was overwritten
                    // by the stored setting on the next launch.
                    void applySettings({ language: value });
                  }}
                >
                  <Languages
                    size={13}
                    className="text-muted"
                    aria-hidden="true"
                  />
                  <span className="flex-1">{label}</span>
                  {locale === value && (
                    <Check
                      size={13}
                      className="text-brand"
                      aria-hidden="true"
                    />
                  )}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
        {dialog}
      </aside>
      <ResizableHandle
        value={sidebarWidth}
        min={SIDEBAR_MIN_WIDTH}
        max={SIDEBAR_MAX_WIDTH}
        label={t("サイドバーの幅を変更")}
        onChange={setSidebarWidth}
        onCommit={commitSidebarWidth}
      />
    </>
  );
}
