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
import { useUi, type View } from "@/lib/ui";
import type { ConversationSummary } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

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
  const modelId = useUi((s) => s.modelId);
  const mode = useUi((s) => s.mode);
  const locale = useUi((s) => s.locale);
  const setLocale = useUi((s) => s.setLocale);
  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);
  const sidebarOpen = useUi((s) => s.sidebarOpen);
  const activeProjectId = useUi((s) => s.activeProjectId);
  const setActiveProjectId = useUi((s) => s.setActiveProjectId);
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");

  const conversations = useQuery({ queryKey: ["conversations"], queryFn: api.listConversations });
  const projects = useQuery({ queryKey: ["projects"], queryFn: api.listProjects });
  const approvals = useQuery({
    queryKey: ["pending_approvals"],
    queryFn: api.listPendingApprovals,
    refetchInterval: 4000,
  });

  const activeProject = projects.data?.find((p) => p.id === activeProjectId) ?? null;

  const createConversation = useMutation({
    mutationFn: () =>
      api.createConversation({
        mode,
        model_id: modelId,
        title: null,
        workspace_id: null,
        project_id: activeProjectId,
      }),
    onSuccess: (created) => {
      void queryClient.invalidateQueries({ queryKey: ["conversations"] });
      setConversationId(created.id);
      setView("chat");
    },
  });

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

  const threads = useMemo(() => {
    const q = search.trim().toLowerCase();
    const list = (conversations.data ?? []).filter(
      (c) =>
        (!activeProjectId || c.project_id === activeProjectId)
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
  }, [conversations.data, search, activeProjectId]);

  const pendingCount = approvals.data?.length ?? 0;
  const hasThreads = (conversations.data?.length ?? 0) > 0;
  const ThreadIcon = THEME_ICON[theme];
  const openThread = (c: ConversationSummary) => {
    setConversationId(c.id);
    if (c.project_id) setActiveProjectId(c.project_id);
    setView("chat");
  };

  return (
    <aside
      data-collapsed={!sidebarOpen}
      aria-label="Sidebar"
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
        </label>

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
                  <h2 className="px-2 pb-1 text-2xs font-semibold uppercase tracking-wide text-faint">
                    {t(GROUP_LABEL[group])}
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
                              <span className="truncate pr-14 text-sm font-medium text-fg">
                                {c.title}
                              </span>
                              {c.preview && (
                                <span className="truncate text-xs text-muted">{c.preview}</span>
                              )}
                              <span className="text-2xs text-faint">
                                {relativeTime(c.updated_at)} · {c.mode}
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
            {(conversations.data?.length ?? 0) === 0 && search && (
              <p className="px-2 text-xs text-faint">{t("スレッドがありません")}</p>
            )}
          </div>
        )}

        {!hasThreads && !conversations.isPending && (
          <p className="px-2 text-xs leading-5 text-faint">
            {t("スレッドがありません")}
            <br />
            {t("新しい会話を始めましょう")}
          </p>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-1.5 border-t border-line px-2 py-1.5">
        <Button
          size="sm"
          variant="ghost"
          onClick={() => setLocale(locale === "ja" ? "en" : "ja")}
          title={t("言語")}
        >
          {locale === "ja" ? "EN" : "JA"}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => setTheme(THEME_NEXT[theme])}
          title={t("テーマ")}
          aria-label={t("テーマ")}
        >
          <ThreadIcon size={14} />
        </Button>
        <span className="flex-1" />
        <span className="text-2xs text-faint">MIX-agent</span>
      </div>
    </aside>
  );
}
