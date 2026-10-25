import { useDeferredValue, useMemo, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  Bot,
  Clock3,
  Download,
  Folder,
  Monitor,
  Moon,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Search,
  Settings2,
  Sun,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { relativeTime, threadGroup, type ThreadGroup } from "@/lib/format";
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, SearchField } from "@/components/ui/input";
import { NavRow, SectionLabel } from "@/components/layout";
import { DeleteIconButton, SearchEmptyState } from "@/components/common";
import { EmptyState, ErrorBox, ListSkeleton } from "@/components/shared";
import { useCreateThread } from "./useShellEvents";
import { panelStyle } from "./usePanelCap";

const NAV_ITEMS: { id: View; label: string; icon: React.ReactNode }[] = [
  { id: "projects", label: "プロジェクト", icon: <Folder size={15} /> },
  { id: "agents", label: "エージェント", icon: <Bot size={15} /> },
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

const THEME_NEXT: Record<
  "light" | "dark" | "system",
  "light" | "dark" | "system"
> = {
  light: "dark",
  dark: "system",
  system: "light",
};

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
  const createConversation = useCreateThread();

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
      const path = await saveDialog({
        defaultPath: `${safe}.md`,
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
          onClick={() => createConversation.mutate()}
          disabled={createConversation.isPending}
        >
          {isAgent ? <Bot size={15} /> : <Plus size={15} />}
          {isAgent ? t("新規タスク") : t("新規チャット")}
        </Button>
        {/* Thread creation failures used to be silent — the button just
            stopped spinning and nothing appeared. */}
        {createConversation.isError && (
          <ErrorBox
            error={
              createConversation.error ||
              (isAgent
                ? t("タスクを作成できませんでした")
                : t("スレッドを作成できませんでした"))
            }
          />
        )}
        <SearchField
          icon={<Search size={13} />}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={isAgent ? t("タスクを検索") : t("チャットを検索")}
          aria-label={isAgent ? t("タスクを検索") : t("チャットを検索")}
          {...(search
            ? { onClear: () => setSearch(""), clearLabel: t("クリア") }
            : {})}
        />
        {exportError && <ErrorBox error={exportError} />}
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
                            "group relative flex flex-col gap-1 rounded-lg py-2 pe-2 ps-2.5",
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
                                <span className="truncate text-sm font-medium text-fg">
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
                                <span>{relativeTime(c.updated_at)}</span>
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

                              <span
                                className={cn(
                                  "absolute end-1 top-1.5 z-10 flex gap-0.5 rounded-md bg-sidebar/90 p-0.5 shadow-xs backdrop-blur-sm",
                                  "opacity-0 transition-opacity duration-100",
                                  "group-hover:opacity-100 group-focus-within:opacity-100 focus-within:opacity-100",
                                  conversationId === c.id && "opacity-100",
                                )}
                              >
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  title={
                                    pinnedSet.has(c.id)
                                      ? t("ピン留めを解除")
                                      : t("ピン留め")
                                  }
                                  aria-label={
                                    pinnedSet.has(c.id)
                                      ? t("ピン留めを解除")
                                      : t("ピン留め")
                                  }
                                  onClick={() => togglePin(c.id)}
                                  className={cn(
                                    pinnedSet.has(c.id) &&
                                      "text-brand hover:text-brand",
                                  )}
                                >
                                  {pinnedSet.has(c.id) ? (
                                    <PinOff size={13} />
                                  ) : (
                                    <Pin size={13} />
                                  )}
                                </Button>
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
                                  <Pencil size={13} />
                                </Button>
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  title={t("エクスポート")}
                                  aria-label={t("エクスポート")}
                                  onClick={() => void exportThread(c)}
                                >
                                  <Download size={13} />
                                </Button>
                                <DeleteIconButton
                                  title={t("スレッドを削除しますか？")}
                                  description={c.title}
                                  onDelete={() => remove.mutate(c.id)}
                                />
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
        {isAgent && (
          <>
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
          </>
        )}
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
          title={`${t("言語")}: ${locale === "ja" ? "日本語" : "English"}`}
          aria-label={`${t("言語")}: ${locale === "ja" ? "日本語" : "English"}`}
          className="px-2 font-mono text-xs"
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
          title={`${t("テーマ")}: ${theme === "light" ? t("ライト") : theme === "dark" ? t("ダーク") : t("システム")}`}
          aria-label={`${t("テーマ")}: ${theme === "light" ? t("ライト") : theme === "dark" ? t("ダーク") : t("システム")}`}
          className="px-2"
        >
          <ThemeIcon size={12} aria-hidden="true" />
        </Button>
      </div>
    </aside>
  );
}
