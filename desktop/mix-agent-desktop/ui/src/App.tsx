import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  FolderTree,
  PanelLeft,
  PanelRight,
  Search,
  Sparkles,
} from "lucide-react";
import { api } from "./lib/api";
import { errorMessage } from "./lib/errors";
import { useUi, type View } from "./lib/ui";
import { t } from "./lib/i18n";
import { normalizeTheme, storedTheme } from "./lib/theme";
import { useConversations, useProjects } from "./lib/queries";
import { SETTINGS_QUERY_KEY } from "./lib/settings";

import { ChatPane } from "./features/chat/ChatPane";
import { AgentsPane } from "./features/agents/AgentsPane";
import { ProjectsPane } from "./features/projects/ProjectsPane";
import { SchedulesPane } from "./features/schedules/SchedulesPane";
import { SettingsPane } from "./features/settings/SettingsPane";
import { FileTreePane } from "./features/files/FileTreePane";
import { Sidebar } from "./features/shell/Sidebar";
import { RightPanel } from "./features/shell/RightPanel";
import { CommandPalette } from "./features/shell/CommandPalette";
import { ShortcutHelp } from "./features/shell/ShortcutHelp";
import { SurfaceSwitcher } from "./features/shell/SurfaceSwitcher";
import { usePanelCap } from "./features/shell/usePanelCap";
import { Onboarding } from "./features/onboarding/Onboarding";
import { useOnboardingGate } from "./features/onboarding/useOnboardingGate";
import {
  useAppShortcuts,
  useCreateThread,
  useShellEvents,
  useUpdateNotice,
} from "./features/shell/useShellEvents";
import { Button } from "./components/ui/button";
import { Kbd } from "./components/layout";
import { Alert } from "./components/ui/alert";
import { Spinner } from "./components/ui/spinner";
import { StatusDot } from "./components/ui/status";

const VIEW_TITLES: Record<View, string> = {
  chat: "チャット",
  agents: "エージェント",
  projects: "プロジェクト",
  schedules: "スケジュール",
  settings: "設定",
};

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
    state === "ok"
      ? t("正常")
      : state === "error"
        ? t("接続エラー")
        : t("確認中");
  return (
    <span title={`${t("ヘルス")}: ${label}`}>
      <StatusDot
        tone={
          state === "ok" ? "success" : state === "error" ? "danger" : "warning"
        }
        label={`${t("ヘルス")}: ${label}`}
        className="px-1"
      />
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
  const setPaletteOpen = useUi((s) => s.setPaletteOpen);
  const openSettings = useUi((s) => s.openSettings);
  const setTheme = useUi((s) => s.setTheme);
  const setLocale = useUi((s) => s.setLocale);
  const conversationId = useUi((s) => s.conversationId);
  const surface = useUi((s) => s.surface);

  useLayoutEffect(() => {
    document.documentElement.dataset.platform = /Macintosh|Mac OS X/i.test(
      navigator.userAgent,
    )
      ? "macos"
      : "other";
  }, []);

  // Sync persisted settings (language/theme) once at boot. Later changes go
  // through `useApplySettings`, which writes both sides at once, so this can
  // no longer fight a language the user just picked in the sidebar.
  const settings = useQuery({
    queryKey: SETTINGS_QUERY_KEY,
    queryFn: api.getSettings,
    staleTime: Infinity,
  });
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
  // The sidebar, the file tree and the right panel are all `shrink-0`, so the
  // shell row — not any one pane — is what decides how much width the side
  // panels may take from the centre column.
  const [shellRow, setShellRow] = useState<HTMLDivElement | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);
  const panelCap = usePanelCap(shellRow);
  useShellEvents({
    newThread: () => createThread.mutate(),
    openView: (target) => setView(target),
    openSettingsTab: (tab) => openSettings(tab),
  });
  useAppShortcuts();

  const activeProjectId = useUi((s) => s.activeProjectId);
  const conversations = useConversations();
  const projects = useProjects();
  const onboarding = useOnboardingGate();
  const activeProject =
    projects.data?.find((p) => p.id === activeProjectId) ?? null;
  const threadTitle = useMemo(
    () => conversations.data?.find((c) => c.id === conversationId)?.title,
    [conversations.data, conversationId],
  );

  // Every hook above runs unconditionally; only the *render* is replaced. A
  // conditional hook call would put the shell's queries out of step with the
  // wizard's for as long as it is on screen.
  //
  // While the gate is undecided nothing is rendered at all. Showing the shell
  // and then replacing it a frame later is the one thing that looks like a bug,
  // and the wait is a single IPC round trip.
  if (!onboarding.ready) {
    return (
      <div className="flex h-full items-center justify-center bg-bg">
        <Spinner size={16} label={t("読み込み中…")} />
      </div>
    );
  }

  if (onboarding.show) {
    return (
      <Onboarding
        onComplete={onboarding.complete}
        onDefer={onboarding.complete}
      />
    );
  }

  const agentSurface = surface === "desktop-agent";

  return (
    <div className="flex h-full min-w-0 flex-col bg-bg">
      <header
        className="app-titlebar relative z-20 flex h-titlebar shrink-0 items-center gap-2 border-b border-line bg-bg/80 px-3 backdrop-blur"
        data-tauri-drag-region="deep"
      >
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={toggleSidebar}
          title={t("サイドバーの表示切り替え")}
          aria-label={t("サイドバーの表示切り替え")}
        >
          <PanelLeft size={16} className={sidebarOpen ? "" : "rotate-180"} />
        </Button>
        <SurfaceSwitcher />
        <span className="mx-0.5 h-4 w-px shrink-0 bg-line" aria-hidden="true" />
        <span className="min-w-0 truncate text-sm font-medium text-fg">
          {view === "chat"
            ? (threadTitle ?? t("チャット"))
            : t(VIEW_TITLES[view])}
        </span>
        {view === "chat" && activeProject && (
          <span
            className="hidden max-w-40 shrink-0 items-center gap-1 truncate text-xs text-faint sm:inline-flex"
            title={activeProject.rootPath ?? undefined}
          >
            <span className="text-line">·</span>
            {activeProject.name}
          </span>
        )}

        <span className="flex-1" />

        <button
          type="button"
          onClick={() => setPaletteOpen(true)}
          aria-label={t("コマンドを検索…")}
          title={`${t("コマンドパレット")} · ⌘K`}
          className={
            "hidden h-7 items-center gap-2 rounded-lg border border-line bg-inset pe-1.5 ps-2.5 " +
            "text-xs text-faint transition-colors duration-100 hover:border-line-strong hover:text-muted md:flex"
          }
        >
          <Search size={13} />
          <span>{t("検索")}</span>
          <Kbd className="ms-2">⌘K</Kbd>
        </button>

        <HealthIndicator />

        {view === "chat" && agentSurface && (
          <div className="flex items-center gap-1 border-s border-line ps-1.5">
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={toggleFileTree}
              title={`${t("ファイルツリー")} · ⌘⇧E`}
              aria-label={t("ファイルツリー")}
              aria-pressed={fileTreeOpen}
            >
              <FolderTree size={15} />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() =>
                setRightPanel(rightPanel === "none" ? "activity" : "none")
              }
              title={
                rightPanel === "none"
                  ? t("アクティビティを表示")
                  : t("サイドパネルを閉じる")
              }
              aria-label={
                rightPanel === "none"
                  ? t("アクティビティを表示")
                  : t("サイドパネルを閉じる")
              }
              aria-pressed={rightPanel !== "none"}
            >
              <PanelRight size={15} />
            </Button>
          </div>
        )}
      </header>

      <div ref={setShellRow} className="flex min-h-0 min-w-0 flex-1">
        <Sidebar cap={panelCap} />
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
                onClick={() => {
                  setInstallError(null);
                  // A failed install (signature mismatch, offline, no write
                  // access) used to reject as an unhandled promise; surface it
                  // under the banner instead of leaving the button inert.
                  void api
                    .installUpdate()
                    .catch((error) => setInstallError(errorMessage(error)));
                }}
              >
                {t("インストールして再起動")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setInstallError(null);
                  dismiss();
                }}
              >
                {t("閉じる")}
              </Button>
            </div>
          )}
          {installError && (
            <div className="shrink-0 border-b border-line px-3 py-2">
              <Alert tone="danger">{installError}</Alert>
            </div>
          )}

          <div className="flex min-h-0 flex-1 flex-row">
            {view === "chat" && agentSurface && fileTreeOpen && (
              <FileTreePane cap={panelCap} />
            )}
            <div className="flex min-h-0 min-w-0 flex-1 flex-col">
              {view === "chat" && <ChatPane />}
              {view === "agents" && <AgentsPane />}
              {view === "projects" && <ProjectsPane />}
              {view === "schedules" && <SchedulesPane />}
              {view === "settings" && <SettingsPane />}
            </div>
          </div>
        </section>
        {view === "chat" && agentSurface && <RightPanel cap={panelCap} />}
      </div>
      <CommandPalette />
      <ShortcutHelp />
    </div>
  );
}
