import { useDeferredValue, useMemo, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Bot,
  Check,
  ChevronDown,
  Clock3,
  Download,
  Folder,
  Languages,
  Monitor,
  Moon,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Search,
  Settings2,
  Sun,
  Trash2,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import {
  absoluteTime,
  relativeTime,
  threadGroup,
  type ThreadGroup,
} from "@/lib/format";
import { useNow } from "@/lib/clock";
import { loadPinnedThreadIds, togglePinnedThreadId } from "@/lib/pinnedThreads";
import { useUi, surfaceForMode, type View } from "@/lib/ui";
import type { ConversationSummary } from "@/lib/types";
import {
  CONVERSATIONS_QUERY_KEY,
  useConversations,
  usePendingApprovals,
  useProjects,
} from "@/lib/queries";
import { useApplySettings } from "@/lib/settings";
import { isBusy } from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import { Badge } from "@/components/ui/badge";
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
import { Input, SearchField } from "@/components/ui/input";
import { NavRow, SectionLabel } from "@/components/layout";
import { SearchEmptyState } from "@/components/common";
import { EmptyState, ErrorBox, ListSkeleton } from "@/components/shared";
import { useNewThread } from "./useShellEvents";
import { panelStyle } from "./usePanelCap";

const NAV_ITEMS: { id: View; label: string; icon: React.ReactNode }[] = [
  { id: "projects", label: "プロジェクト", icon: <Folder size={15} /> },
  {
    // Not "エージェント": that word is the workspace and the run mode. This
    // page is the list of saved agent profiles.
    id: "agents",
    label: "エージェントプロファイル",
    icon: <Bot size={15} />,
  },
  { id: "schedules", label: "スケジュール", icon: <Clock3 size={15} /> },
];

type ExtendedThreadGroup = "pinned" | ThreadGroup;
const GROUP_ORDER: ExtendedThreadGroup[] = [
  "pinned",
  "today",
  "yesterday",
  "week",
  "older",
];
const GROUP_LABEL: Record<ExtendedThreadGroup, string> = {
  pinned: "ピン留め済み",
  today: "今日",
  yesterday: "昨日",
  week: "過去7日",
  older: "それより前",
};

/** Run modes, shown as words rather than the wire values. */
const MODE_LABEL: Record<string, string> = {
  chat: "チャット",
  thinking: "思考",
  agent: "エージェント",
};

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

/** Fixed Codex-like width. The sidebar used to be resizable (210–440px) with a
 * drag handle, but every drag pixel reflowed the centre column and flipped the
 * composer's container-query toolbar mid-drag — the classic "unstable" feel.
 * A fixed width keeps the transcript and composer geometry constant. */
const SIDEBAR_WIDTH = 264;

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
  const activeProjectId = useUi((s) => s.activeProjectId);
  const applySettings = useApplySettings();
  const queryClient = useQueryClient();
  const { confirm, dialog } = useConfirm();
  const now = useNow();
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
  /** Ref for arrow-key travel over the thread titles (see below). */
  const threadListRef = useRef<HTMLDivElement | null>(null);

  const isAgent = surface === "desktop-agent";
  const projects = useProjects();
  const activeProject =
    projects.data?.find((p) => p.id === activeProjectId) ?? null;
  const conversations = useConversations();
  const approvals = usePendingApprovals();
  const newThread = useNewThread();
  // A run keeps streaming after the user opens another thread. Without this
  // marker the work simply vanished from the UI: the transcript stopped
  // receiving events and nothing else in the shell said a task was still live.
  const liveByConversation = useRunEvents((s) => s.byConversation);
  const runningConversationIds = useMemo(
    () =>
      new Set(
        Object.entries(liveByConversation)
          .filter(([, entry]) => isBusy(entry.timeline.status))
          .map(([id]) => id),
      ),
    [liveByConversation],
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
    const list = (conversations.data ?? []).filter((c) => {
      // Natural separation by active surface:
      if (surfaceForMode(c.mode) !== surface) {
        return false;
      }
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
  }, [conversations.data, deferredSearch, pinnedSet, surface]);

  const pendingCount = approvals.data?.length ?? 0;
  const hasThreads = (conversations.data ?? []).some(
    (c) => surfaceForMode(c.mode) === surface,
  );
  const filteredThreadCount = [...threads.values()].reduce(
    (n, items) => n + items.length,
    0,
  );
  const ThemeIcon = THEME_ICON[theme];
  const openThread = (c: ConversationSummary) => {
    setConversationId(c.id);
    setView("chat");
  };

  /**
   * Arrow-key travel between thread titles.
   *
   * Each title is already a real `<button>` (so Tab and Enter work), but a long
   * history is dozens of tab stops. Once focus is on a title, Up/Down move to
   * the neighbouring thread and Home/End jump to the ends of the visible list.
   * The guard on `[data-thread-open]` keeps the keys working as caret movement
   * while the rename field has focus.
   */
  const onThreadListKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (
      event.key !== "ArrowDown" &&
      event.key !== "ArrowUp" &&
      event.key !== "Home" &&
      event.key !== "End"
    ) {
      return;
    }
    const target = event.target as HTMLElement;
    if (!target.matches("[data-thread-open]")) return;
    const buttons = Array.from(
      threadListRef.current?.querySelectorAll<HTMLButtonElement>(
        "[data-thread-open]",
      ) ?? [],
    );
    const index = buttons.indexOf(target as HTMLButtonElement);
    if (index === -1) return;
    event.preventDefault();
    const last = buttons.length - 1;
    const next =
      event.key === "ArrowDown"
        ? Math.min(index + 1, last)
        : event.key === "ArrowUp"
          ? Math.max(index - 1, 0)
          : event.key === "Home"
            ? 0
            : last;
    buttons[next]?.focus();
  };

  // Codex-like: instant show/hide with no width animation. The old
  // negative-margin transition animated the centre column's width for 150ms
  // on every toggle, and the panel-cap observer re-measured mid-animation so
  // the panes visibly jumped. Unmounted when closed: nothing to measure.
  if (!sidebarOpen) {
    return null;
  }

  return (
    <aside
      aria-label={t("サイドバー")}
      data-side-panel=""
      // Fixed width; `cap` only clamps it on very narrow windows.
      style={panelStyle(SIDEBAR_WIDTH, cap)}
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
              ? t("タスクのタイトル・プレビューを検索")
              : t("チャットのタイトル・プレビューを検索")
          }
          aria-label={
            isAgent
              ? t("タスクのタイトル・プレビューを検索")
              : t("チャットのタイトル・プレビューを検索")
          }
          {...(search
            ? { onClear: () => setSearch(""), clearLabel: t("クリア") }
            : {})}
        />
        {(exportError || rename.isError || remove.isError) && (
          <ErrorBox error={exportError ?? rename.error ?? remove.error} />
        )}
      </div>

      {/* Thread history — the main content of the sidebar. */}
      {/* Key delegation: titles are native buttons (Tab/Enter free); arrows
          move between them from one handler instead of N. */}
      {/* eslint-disable-next-line jsx-a11y/no-static-element-interactions */}
      <div
        ref={threadListRef}
        onKeyDown={onThreadListKeyDown}
        className="stable-scroll flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-3 pt-1.5"
      >
        {hasThreads ? (
          <>
            {GROUP_ORDER.map((group) => {
              const items = threads.get(group);
              if (!items?.length) return null;
              return (
                <section key={group}>
                  <SectionLabel
                    trailing={
                      <span className="font-mono font-normal text-2xs text-faint">
                        {items.length}
                      </span>
                    }
                  >
                    {t(GROUP_LABEL[group])}
                  </SectionLabel>
                  <ul className="flex flex-col gap-0.5">
                    {items.map((c) => (
                      <li key={c.id}>
                        {/* A real `<button>`, not a `div role="button"`. The row
                            also holds rename / export / delete buttons, so the
                            title stretches over the row with a pseudo-element. */}
                        <div
                          className={cn(
                            "group relative flex flex-col gap-1 rounded-lg py-2 pe-9 ps-2.5",
                            "transition-colors duration-100 hover:bg-hover",
                            conversationId === c.id &&
                              "bg-brand-soft ring-1 ring-brand/20 ring-inset",
                          )}
                        >
                          {renaming === c.id ? (
                            <Input
                              className="h-7 px-2 text-sm"
                              value={renameDraft}
                              // Mounted only while renaming: focus must enter
                              // the field or keyboard users are stranded.
                              // eslint-disable-next-line jsx-a11y/no-autofocus
                              autoFocus
                              onChange={(e) => setRenameDraft(e.target.value)}
                              onKeyDown={(e) => {
                                e.stopPropagation();
                                if (e.key === "Enter" && renameDraft.trim()) {
                                  // Commit through blur so Enter + blur cannot
                                  // fire the rename twice.
                                  e.currentTarget.blur();
                                }
                                if (e.key === "Escape") setRenaming(null);
                              }}
                              onBlur={() => {
                                commitRename(c.id, c.title);
                              }}
                            />
                          ) : (
                            <>
                              <button
                                type="button"
                                data-thread-open
                                onClick={() => openThread(c)}
                                aria-current={
                                  conversationId === c.id ? "true" : undefined
                                }
                                className="flex min-w-0 items-center gap-1.5 text-start after:absolute after:inset-0 after:z-0 after:content-['']"
                              >
                                {c.project_id && (
                                  <span
                                    className="size-1.5 shrink-0 rounded-full bg-brand"
                                    aria-hidden="true"
                                    title={t("プロジェクト紐付けあり")}
                                  />
                                )}
                                {runningConversationIds.has(c.id) && (
                                  <span
                                    role="img"
                                    aria-label={t("実行中")}
                                    title={t("実行中")}
                                    className="size-1.5 shrink-0 animate-pulse rounded-full bg-success"
                                  />
                                )}
                                <span
                                  className="truncate text-sm font-medium text-fg"
                                  title={c.title}
                                >
                                  {c.title}
                                </span>
                                {pinnedSet.has(c.id) && (
                                  <Pin
                                    size={12}
                                    className="shrink-0 text-brand fill-brand/30"
                                    aria-hidden="true"
                                  />
                                )}
                              </button>
                              {c.preview && (
                                <span className="pointer-events-none truncate text-xs text-muted">
                                  {c.preview}
                                </span>
                              )}
                              <span className="pointer-events-none flex items-center gap-1 text-xs text-faint">
                                <time
                                  dateTime={c.updated_at}
                                  title={absoluteTime(c.updated_at)}
                                >
                                  {relativeTime(c.updated_at, new Date(now))}
                                </time>
                                <span className="text-line">·</span>
                                <span>{t(MODE_LABEL[c.mode] ?? c.mode)}</span>
                                {c.message_count > 0 && (
                                  <>
                                    <span className="text-line">·</span>
                                    <span className="font-mono">
                                      {c.message_count}
                                    </span>
                                  </>
                                )}
                              </span>

                              <DropdownMenu>
                                <DropdownMenuTrigger asChild>
                                  <Button
                                    variant="ghost"
                                    size="icon-sm"
                                    title={t("スレッド操作")}
                                    aria-label={t("スレッド操作")}
                                    className={cn(
                                      "absolute end-1 top-1.5 z-10 bg-sidebar/90 shadow-xs backdrop-blur-sm",
                                      "opacity-0 transition-opacity duration-100",
                                      "group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100",
                                      conversationId === c.id && "opacity-100",
                                    )}
                                  >
                                    <MoreHorizontal size={14} />
                                  </Button>
                                </DropdownMenuTrigger>
                                <DropdownMenuContent
                                  align="end"
                                  className="min-w-44"
                                >
                                  <DropdownMenuItem
                                    onSelect={() => togglePin(c.id)}
                                  >
                                    {pinnedSet.has(c.id) ? (
                                      <PinOff
                                        size={13}
                                        className="text-muted"
                                      />
                                    ) : (
                                      <Pin size={13} className="text-muted" />
                                    )}
                                    <span className="flex-1">
                                      {pinnedSet.has(c.id)
                                        ? t("ピン留めを解除")
                                        : t("ピン留め")}
                                    </span>
                                  </DropdownMenuItem>
                                  <DropdownMenuItem
                                    onSelect={() => {
                                      setRenaming(c.id);
                                      setRenameDraft(c.title);
                                    }}
                                  >
                                    <Pencil size={13} className="text-muted" />
                                    <span className="flex-1">
                                      {t("名前を変更")}
                                    </span>
                                  </DropdownMenuItem>
                                  <DropdownMenuItem
                                    onSelect={() => void exportThread(c)}
                                  >
                                    <Download
                                      size={13}
                                      className="text-muted"
                                    />
                                    <span className="flex-1">
                                      {t("エクスポート")}
                                    </span>
                                  </DropdownMenuItem>
                                  <DropdownMenuSeparator />
                                  <DropdownMenuItem
                                    disabled={remove.isPending}
                                    onSelect={() => void removeThread(c)}
                                    className="text-danger focus:bg-danger-soft focus:text-danger"
                                  >
                                    <Trash2 size={13} />
                                    <span className="flex-1">
                                      {remove.isPending
                                        ? t("削除中…")
                                        : t("削除")}
                                    </span>
                                  </DropdownMenuItem>
                                </DropdownMenuContent>
                              </DropdownMenu>
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
              <SearchEmptyState
                compact
                title={
                  isAgent
                    ? t("一致するタスクがありません")
                    : t("一致するチャットがありません")
                }
                onClear={() => setSearch("")}
              />
            )}
          </>
        ) : conversations.isError ? (
          <ErrorBox error={conversations.error || t("エラー")} />
        ) : conversations.isPending ? (
          <ListSkeleton rows={3} label={t("読み込み中…")} />
        ) : (
          <EmptyState
            compact
            title={
              isAgent ? t("タスクがありません") : t("チャット履歴がありません")
            }
            sub={
              isAgent
                ? t("新しいタスクを始めましょう")
                : t("新しい会話を始めましょう")
            }
          />
        )}
      </div>

      {/* Secondary destinations, below the history so threads stay primary. */}
      <nav className="flex shrink-0 flex-col gap-0.5 border-t border-line p-3 pt-2.5">
        {/* Shown on both workspaces. They were agent-only, yet ⌘K opened them
            from Chat too — where the sidebar then hid their only navigation and
            the page had no way back. */}
        <SectionLabel>{t("ワークスペース")}</SectionLabel>
        {NAV_ITEMS.map((item) => (
          <NavRow
            key={item.id}
            icon={item.icon}
            selected={view === item.id}
            onClick={() => setView(item.id)}
          >
            {t(item.label)}
          </NavRow>
        ))}
        <NavRow
          icon={<Settings2 size={15} />}
          selected={view === "settings"}
          onClick={() => setView("settings")}
          trailing={
            pendingCount > 0 ? (
              <Badge tone="warning" size="sm">
                {pendingCount}
              </Badge>
            ) : undefined
          }
        >
          {t("設定")}
        </NavRow>
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
                  <Check size={13} className="text-brand" aria-hidden="true" />
                )}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {dialog}
    </aside>
  );
}
