import { useMemo, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  Bot,
  Clock3,
  Download,
  Folder,
  MessageSquare,
  Monitor,
  Moon,
  Pencil,
  Plus,
  Search,
  Settings2,
  Sun,
  Trash2,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { relativeTime, threadGroup, type ThreadGroup } from "@/lib/format";
import { useUi, surfaceForMode, type View, type ThreadFilter } from "@/lib/ui";
import type { ConversationSummary } from "@/lib/types";
import { errorMessage } from "@/lib/errors";
import {
  CONVERSATIONS_QUERY_KEY,
  useConversations,
  usePendingApprovals,
} from "@/lib/queries";
import { useApplySettings } from "@/lib/settings";
import { Alert, Skeleton } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Input, SearchField } from "@/components/ui/input";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { NavRow, SectionLabel } from "@/components/layout";
import { ResizableHandle } from "./ResizableHandle";
import { useCreateThread } from "./useShellEvents";
import { panelStyle, panelWidth } from "./usePanelCap";
import { usePanelWidth } from "./usePanelWidth";

const NAV_ITEMS: { id: View; label: string; icon: React.ReactNode }[] = [
  { id: "projects", label: "プロジェクト", icon: <Folder size={15} /> },
  { id: "agents", label: "エージェント", icon: <Bot size={15} /> },
  { id: "schedules", label: "スケジュール", icon: <Clock3 size={15} /> },
];

const GROUP_ORDER: ThreadGroup[] = ["today", "yesterday", "week", "older"];
const GROUP_LABEL: Record<ThreadGroup, string> = {
  today: "今日",
  yesterday: "昨日",
  week: "過去7日",
  older: "それより前",
};

/** Order and wording of the thread-list filter. `all` first so it is the
 *  default reading; the two surface values mirror `ThreadFilter`. */
const THREAD_FILTERS: ThreadFilter[] = ["all", "chat", "desktop-agent"];
const THREAD_FILTER_LABEL: Record<ThreadFilter, string> = {
  all: "すべて",
  chat: "チャット",
  "desktop-agent": "エージェント",
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
  const threadFilter = useUi((s) => s.threadFilter);
  const setThreadFilter = useUi((s) => s.setThreadFilter);
  const { width, setWidth, commitWidth } = usePanelWidth(
    "mix-agent-sidebar-width",
    264,
    210,
    440,
  );
  const applySettings = useApplySettings();
  const { confirm, dialog } = useConfirm();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [exportError, setExportError] = useState<string | null>(null);
  /** Ref for arrow-key travel over the thread titles (see below). */
  const threadListRef = useRef<HTMLDivElement | null>(null);

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
    const q = search.trim().toLowerCase();
    const list = (conversations.data ?? []).filter((c) => {
      // Surface filter first, then the free-text search. The store already
      // persisted `threadFilter`; this is the list that finally honours it.
      if (threadFilter !== "all" && surfaceForMode(c.mode) !== threadFilter) {
        return false;
      }
      return (
        !q ||
        c.title.toLowerCase().includes(q) ||
        (c.preview ?? "").toLowerCase().includes(q)
      );
    });
    const groups = new Map<ThreadGroup, ConversationSummary[]>();
    for (const c of list) {
      const group = threadGroup(c.updated_at);
      const bucket = groups.get(group) ?? [];
      bucket.push(c);
      groups.set(group, bucket);
    }
    return groups;
  }, [conversations.data, search, threadFilter]);

  const pendingCount = approvals.data?.length ?? 0;
  const hasThreads = (conversations.data?.length ?? 0) > 0;
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

  return (
    <>
      <aside
        aria-label={t("サイドバー")}
        data-side-panel=""
        // The collapsed margin is a negative one, so it keeps riding on the
        // same custom property; the width itself is inline because that is what
        // `usePanelCap` reads to work out the shortfall.
        style={
          {
            "--sidebar-w": `${width}px`,
            ...panelStyle(width, cap),
          } as React.CSSProperties
        }
        className={cn(
          "flex shrink-0 flex-col overflow-hidden border-r border-line bg-sidebar",
          "transition-[margin] duration-150 ease-out",
          sidebarOpen ? "mr-0" : "-mr-(--sidebar-w)",
        )}
      >
        {/* Primary action + search, pinned so they are always reachable. */}
        <div className="flex shrink-0 flex-col gap-2 p-3 pb-1.5">
          <Button
            variant="primary"
            size="lg"
            className="h-10 w-full justify-start gap-2"
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
            {...(search
              ? { onClear: () => setSearch(""), clearLabel: t("クリア") }
              : {})}
          />
          {/* Surface filter. The store has held `threadFilter` all along; this
              is the control that finally drives it, so an agent-workspace user
              can hide the plain chat threads (and vice versa). */}
          <Segmented
            aria-label={t("スレッドを絞り込む")}
            className="w-full"
          >
            {THREAD_FILTERS.map((filter) => (
              <SegmentedOption
                key={filter}
                value={filter}
                label={t(THREAD_FILTER_LABEL[filter])}
                selected={threadFilter === filter}
                onSelect={setThreadFilter}
              >
                {t(THREAD_FILTER_LABEL[filter])}
              </SegmentedOption>
            ))}
          </Segmented>
          {exportError && (
            <Alert tone="danger" className="text-xs">
              {exportError}
            </Alert>
          )}
        </div>

        {/* Thread history — the main content of the sidebar. */}
        <div
          ref={threadListRef}
          onKeyDown={onThreadListKeyDown}
          className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-3 pt-1.5"
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
                        <span className="font-mono font-normal text-faint/70">
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
                              conversationId === c.id && "bg-brand-soft",
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
                                    rename.mutate({
                                      id: c.id,
                                      title: renameDraft.trim(),
                                    });
                                  }
                                  if (e.key === "Escape") setRenaming(null);
                                }}
                                onBlur={() => {
                                  if (
                                    renameDraft.trim() &&
                                    renameDraft !== c.title
                                  ) {
                                    rename.mutate({
                                      id: c.id,
                                      title: renameDraft.trim(),
                                    });
                                  } else {
                                    setRenaming(null);
                                  }
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
                                      if (
                                        await confirm({
                                          title: t("スレッドを削除しますか？"),
                                          description: c.title,
                                          confirmLabel: t("削除"),
                                          tone: "danger",
                                        })
                                      ) {
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
                <p className="px-2.5 py-2 text-xs text-faint">
                  {t("一致するスレッドがありません")}
                </p>
              )}
            </>
          ) : conversations.isError ? (
            <Alert tone="danger">
              {errorMessage(conversations.error) || t("エラー")}
            </Alert>
          ) : conversations.isPending ? (
            <div aria-hidden className="flex flex-col gap-1.5">
              {[0, 1, 2].map((i) => (
                <div
                  key={i}
                  className="rounded-lg border border-line px-2.5 py-2"
                >
                  <Skeleton className="h-3 w-2/3" />
                  <Skeleton className="mt-1.5 h-2.5 w-1/3" />
                </div>
              ))}
            </div>
          ) : (
            <div className="mt-1 flex flex-col items-center gap-1 rounded-xl border border-dashed border-line px-3 py-8 text-center">
              <MessageSquare size={18} className="text-faint" />
              <span className="text-xs font-medium text-muted">
                {t("スレッドがありません")}
              </span>
              <span className="text-xs leading-4 text-faint">
                {t("新しい会話を始めましょう")}
              </span>
            </div>
          )}
        </div>

        {/* Secondary destinations, below the history so threads stay primary. */}
        <nav className="flex shrink-0 flex-col gap-0.5 border-t border-line p-3 pt-2.5">
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
          <span className="truncate px-1 text-xs font-medium text-faint">
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
            title={t("言語")}
            aria-label={t("言語")}
            className="font-mono text-xs"
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
            <ThemeIcon size={14} />
          </Button>
        </div>
      </aside>
      {sidebarOpen && (
        <ResizableHandle
          // The rendered width, not the remembered one: under a cap the two
          // differ, and a handle reporting a position it is not at makes the
          // drag jump when the pane finally moves.
          value={panelWidth(width, cap)}
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
