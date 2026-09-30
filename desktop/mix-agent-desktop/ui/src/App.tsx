import { useEffect, useLayoutEffect, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { FolderTree, GitBranch, PanelLeft, PanelRight, Sparkles } from "lucide-react";
import { api } from "./lib/api";
import { cn } from "./lib/cn";
import { useUi, type View } from "./lib/ui";
import { t } from "./lib/i18n";
import { normalizeTheme, storedTheme } from "./lib/theme";
import { useConversations, useProjects } from "./lib/queries";

import { ChatPane } from "./features/chat/ChatPane";
import { AgentsPane } from "./features/agents/AgentsPane";
import { ProjectsPane } from "./features/projects/ProjectsPane";
import { SchedulesPane } from "./features/schedules/SchedulesPane";
import { SettingsPane } from "./features/settings/SettingsPane";
import { FileTreePane } from "./features/files/FileTreePane";
import { Sidebar } from "./features/shell/Sidebar";
import { RightPanel } from "./features/shell/RightPanel";
import { CommandPalette } from "./features/shell/CommandPalette";
import { SurfaceSwitcher } from "./features/shell/SurfaceSwitcher";
import {
  useAppShortcuts,
  useCreateThread,
  useShellEvents,
  useUpdateNotice,
} from "./features/shell/useShellEvents";
import { Button } from "./components/ui/button";

const VIEW_TITLES: Record<View, string> = {
  chat: "チャット",
  agents: "エージェント",
  projects: "プロジェクト",
  schedules: "スケジュール",
  settings: "設定",
};

/** The health dot. Colour alone would say nothing to a screen reader, so the
 * state is also carried by `aria-label` and hidden text. */
/** How often the shell asks the backend whether it is still alive. */
const HEALTH_POLL_MS = 15_000;

function HealthIndicator() {
  // The poll lives here, not in `App`. Holding it in the shell meant every
  // 15s tick re-rendered the sidebar, the transcript and the right panel to
  // repaint one two-pixel dot.
  const health = useQuery({
    queryKey: ["health"],
    queryFn: api.health,
    refetchInterval: HEALTH_POLL_MS,
  });
  const state = health.isError
    ? "error"
    : health.data?.status === "ok"
      ? "ok"
      : "unknown";
  const label =
    state === "ok" ? t("正常") : state === "error" ? t("接続エラー") : t("確認中");
  return (
    <span
      role="status"
      aria-label={`${t("ヘルス")}: ${label}`}
      className="inline-flex items-center"
    >
      <span
        aria-hidden="true"
        className={cn(
          "size-2 shrink-0 rounded-full",
          state === "ok" ? "bg-success" : state === "error" ? "bg-danger" : "bg-warning",
        )}
      />
      <span className="sr-only">{label}</span>
    </span>
  );
}

export function App() {
  const view = useUi((s) => s.view);
  const setView = useUi((s) => s.setView);
  const sidebarOpen = useUi((s) => s.sidebarOpen);
  const toggleSidebar = useUi((s) => s.toggleSidebar);
  const rightPanel = useUi((s) => s.rightPanel);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const fileTreeOpen = useUi((s) => s.fileTreeOpen);
  const toggleFileTree = useUi((s) => s.toggleFileTree);
  const openSettings = useUi((s) => s.openSettings);
  const setTheme = useUi((s) => s.setTheme);
  const setLocale = useUi((s) => s.setLocale);
  const conversationId = useUi((s) => s.conversationId);
  const surface = useUi((s) => s.surface);

  useLayoutEffect(() => {
    document.documentElement.dataset.platform = /Macintosh|Mac OS X/i.test(navigator.userAgent)
      ? "macos"
      : "other";
  }, []);

  // Sync persisted settings (language/theme) once at boot. Later changes go
  // through `useApplySettings`, which writes both sides at once, so this can
  // no longer fight a language the user just picked in the sidebar.
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.getSettings, staleTime: Infinity });
  useEffect(() => {
    const data = settings.data;
    if (!data) return;
    if (data.language === "ja" || data.language === "en") {
      const current = useUi.getState().locale;
      if (data.language !== current) setLocale(data.language);
    }
    const theme = normalizeTheme(data.theme);
    if (theme !== storedTheme()) setTheme(theme);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.data]);

  const createThread = useCreateThread();
  const { update, dismiss } = useUpdateNotice();
  useShellEvents({
    newThread: () => createThread.mutate(),
    openView: (target) => setView(target),
    openSettingsTab: (tab) => openSettings(tab),
  });
  useAppShortcuts();

  const activeProjectId = useUi((s) => s.activeProjectId);
  const conversations = useConversations();
  const projects = useProjects();
  const activeProject = projects.data?.find((p) => p.id === activeProjectId) ?? null;
  const threadTitle = useMemo(
    () => conversations.data?.find((c) => c.id === conversationId)?.title,
    [conversations.data, conversationId],
  );

  return (
    <div className="flex h-full min-w-0 flex-col bg-bg">
      <header
        className="app-titlebar relative z-20 flex h-11 shrink-0 items-center gap-2 border-b border-line bg-bg px-3"
        data-tauri-drag-region="deep"
      >
        <Button
          variant="ghost"
          size="icon"
          onClick={toggleSidebar}
          title={t("サイドバーの表示切り替え")}
          aria-label={t("サイドバーの表示切り替え")}
        >
          <PanelLeft size={15} className={sidebarOpen ? "" : "rotate-180"} />
        </Button>
        <SurfaceSwitcher />
        <span className="truncate text-sm font-medium text-fg">
          {view === "chat" ? (threadTitle ?? t("チャット")) : t(VIEW_TITLES[view])}
        </span>
        {view === "chat" && activeProject && (
          <span
            className="truncate text-xs text-muted"
            title={activeProject.rootPath ?? undefined}
          >
            {activeProject.name}
          </span>
        )}
        <span className="flex-1" />
        <HealthIndicator />
        {view === "chat" && surface === "desktop-agent" && (
          <div className="flex items-center gap-0.5 border-s border-line ps-1.5">
            <Button
              variant="ghost"
              size="icon"
              data-active={fileTreeOpen}
              onClick={toggleFileTree}
              title={`${t("ファイルツリー")} · ⌘⇧E`}
              aria-label={t("ファイルツリー")}
              aria-pressed={fileTreeOpen}
            >
              <FolderTree size={15} />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              data-active={rightPanel === "changes"}
              onClick={() => setRightPanel(rightPanel === "changes" ? "activity" : "changes")}
              title={rightPanel === "changes" ? t("アクティビティを表示") : t("変更")}
              aria-label={t("変更")}
              aria-pressed={rightPanel === "changes"}
            >
              <GitBranch size={15} />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              data-active={rightPanel === "activity"}
              onClick={() => setRightPanel(rightPanel === "activity" ? "none" : "activity")}
              title={rightPanel === "activity" ? t("サイドパネルを閉じる") : `${t("このRunのアクティビティ")} · ⌘⌥R`}
              aria-label={t("このRunのアクティビティ")}
              aria-pressed={rightPanel === "activity"}
            >
              <PanelRight size={15} />
            </Button>
          </div>
        )}
      </header>

      <div className="flex min-h-0 min-w-0 flex-1">
        <Sidebar />
        <section className="flex min-w-0 flex-1 flex-col bg-bg">
          {update && (
            <div className="flex shrink-0 items-center gap-3 border-b border-line bg-brand-soft px-3 py-2 text-xs text-brand">
              <Sparkles size={14} className="shrink-0" />
              <span className="flex-1">
                {t("アップデートがあります")}: {update.current_version} →{" "}
                <strong>{update.version}</strong>
              </span>
              <Button
                variant="primary"
                size="sm"
                onClick={() => void api.installUpdate()}
              >
                {t("インストールして再起動")}
              </Button>
              <Button variant="ghost" size="sm" onClick={dismiss}>
                {t("閉じる")}
              </Button>
            </div>
          )}

          <div className="flex min-h-0 flex-1 flex-row">
            {view === "chat" && surface === "desktop-agent" && fileTreeOpen && <FileTreePane />}
            <div className="flex min-h-0 min-w-0 flex-1 flex-col">
              {view === "chat" && <ChatPane />}
              {view === "agents" && <AgentsPane />}
              {view === "projects" && <ProjectsPane />}
              {view === "schedules" && <SchedulesPane />}
              {view === "settings" && <SettingsPane />}
            </div>
          </div>
        </section>
        {view === "chat" && surface === "desktop-agent" && <RightPanel />}
      </div>
      <CommandPalette />
    </div>
  );
}
