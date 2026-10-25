import {
  Suspense,
  lazy,
  useEffect,
  useLayoutEffect,
  useMemo,
  useState,
} from "react";
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
// Secondary panes are code-split: the chat path (the startup view) must not
// wait on the agents/projects/schedules/settings/file-tree code. Each lands
// in its own chunk (see `manualChunks` in vite.config.ts).
const AgentsPane = lazy(() =>
  import("./features/agents/AgentsPane").then((m) => ({
    default: m.AgentsPane,
  })),
);
const ProjectsPane = lazy(() =>
  import("./features/projects/ProjectsPane").then((m) => ({
    default: m.ProjectsPane,
  })),
);
const SchedulesPane = lazy(() =>
  import("./features/schedules/SchedulesPane").then((m) => ({
    default: m.SchedulesPane,
  })),
);
const SettingsPane = lazy(() =>
  import("./features/settings/SettingsPane").then((m) => ({
    default: m.SettingsPane,
  })),
);
const FileTreePane = lazy(() =>
  import("./features/files/FileTreePane").then((m) => ({
    default: m.FileTreePane,
  })),
);
import { Sidebar } from "./features/shell/Sidebar";
import { RightPanel } from "./features/shell/RightPanel";
import { CommandPalette } from "./features/shell/CommandPalette";
import { ShortcutHelp } from "./features/shell/ShortcutHelp";
import { SurfaceSwitcher } from "./features/shell/SurfaceSwitcher";
import { WindowControls } from "./features/shell/WindowControls";
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
import { ErrorBox } from "./components/shared";
import { PaneErrorBoundary } from "./components/PaneErrorBoundary";
import { Spinner } from "./components/ui/spinner";
import { StatusDot } from "./components/ui/status";

/** Fallback while a code-split pane loads (see the `lazy` imports above). */
function PaneFallback() {
  return (
    <div
      className="flex min-h-0 flex-1 items-center justify-center"
      role="status"
      aria-busy="true"
    >
      <Spinner size={16} label={t("読み込み中…")} />
    </div>
  );
}

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
    <span className="inline-flex items-center">
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
    // Tauri の WebView では UA が変わることがあり、UA  sniffing だけだと
    // macOS でも "other" になって traffic light とボタンが重なる。
    // plugin-os が使える時はそちらを正とする。
    let cancelled = false;
    const uaFallback = /Macintosh|Mac OS X/i.test(navigator.userAgent)
      ? "macos"
      : "other";
    document.documentElement.dataset.platform = uaFallback;
    (async () => {
      try {
        const { platform } = await import("@tauri-apps/plugin-os");
        const os = await platform();
        if (!cancelled) {
          document.documentElement.dataset.platform =
            os === "macos" ? "macos" : "other";
        }
      } catch {
        // ブラウザプレビュー等では plugin が無いので UA 判定のまま。
      }
    })();
    return () => {
      cancelled = true;
    };
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
  const {
    update,
    isInstalling,
    downloadProgress,
    error: installError,
    install: installUpdate,
    dismiss,
  } = useUpdateNotice();
  // The sidebar and the file tree are fixed-width `shrink-0` panes; the right
  // panel is an overlay drawer, so opening it never takes width from the
  // centre column. `panelCap` only clamps the inline panes on narrow windows.
  const [shellRow, setShellRow] = useState<HTMLDivElement | null>(null);
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
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:absolute focus:top-1 focus:left-1 focus:z-50 focus:rounded-md focus:bg-surface focus:px-3 focus:py-1.5 focus:text-sm focus:shadow-pop"
        onClick={(e) => {
          e.preventDefault();
          document.getElementById("main-content")?.focus();
        }}
      >
        {t("メインコンテンツへ移動")}
      </a>
      <header
        className="app-titlebar relative z-20 flex h-titlebar shrink-0 items-center gap-1.5 border-b border-line bg-bg/85 px-3 backdrop-blur-md"
        data-tauri-drag-region="deep"
      >
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={toggleSidebar}
          title={`${t("サイドバーの表示切り替え")} · ⌘B`}
          aria-label={t("サイドバーの表示切り替え")}
          aria-pressed={sidebarOpen}
        >
          <PanelLeft size={16} className={sidebarOpen ? "" : "rotate-180"} />
        </Button>
        <SurfaceSwitcher />
        <span className="mx-0.5 h-4 w-px shrink-0 bg-line" aria-hidden="true" />
        <h1 className="min-w-0 flex-1 truncate text-sm font-medium text-fg">
          {view === "chat"
            ? (threadTitle ?? (agentSurface ? t("タスク") : t("チャット")))
            : t(VIEW_TITLES[view])}
          {view === "chat" && agentSurface && activeProject && (
            <span
              className="ms-2 hidden max-w-40 truncate align-middle text-xs font-normal text-faint sm:inline-block"
              title={activeProject.rootPath ?? undefined}
            >
              <span className="me-1 text-line">·</span>
              {activeProject.name}
            </span>
          )}
        </h1>

        <div className="flex shrink-0 items-center gap-1.5">
          <button
            type="button"
            onClick={() => setPaletteOpen(true)}
            aria-label={t("コマンドを検索…")}
            aria-keyshortcuts="meta+k ctrl+k"
            title={`${t("コマンドパレット")} · ⌘K`}
            className={
              "hidden h-7 min-w-44 items-center gap-2 rounded-lg border border-line bg-surface px-2.5 " +
              "text-xs text-faint shadow-xs transition-colors duration-100 hover:border-line-strong hover:text-muted md:flex"
            }
          >
            <Search size={13} aria-hidden="true" />
            <span className="flex-1 text-start">{t("検索")}</span>
            <Kbd>⌘K</Kbd>
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
                <FolderTree size={16} />
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
                <PanelRight size={16} />
              </Button>
            </div>
          )}
          <WindowControls />
        </div>
      </header>

      <div ref={setShellRow} className="relative flex min-h-0 min-w-0 flex-1">
        <Sidebar cap={panelCap} />
        <section
          id="main-content"
          tabIndex={-1}
          aria-label={
            view === "chat"
              ? (threadTitle ?? t("チャット"))
              : t(VIEW_TITLES[view])
          }
          className="flex min-w-0 flex-1 flex-col bg-bg outline-none"
        >
          {update && (
            <div
              className="flex shrink-0 flex-col border-b border-brand/20 bg-brand-soft px-4 py-2 text-xs text-fg"
              role="status"
            >
              <div className="flex items-center gap-3">
                <span className="grid size-6 shrink-0 place-items-center rounded-md bg-brand/15 text-brand">
                  <Sparkles size={14} aria-hidden="true" />
                </span>
                <span className="min-w-0 flex-1 truncate">
                  {t("アップデートがあります")}: {update.current_version} →{" "}
                  <strong className="font-semibold">{update.version}</strong>
                </span>
                <Button
                  variant="primary"
                  size="sm"
                  disabled={isInstalling}
                  onClick={() => {
                    void installUpdate().catch(() => {});
                  }}
                >
                  {isInstalling ? (
                    <span className="flex items-center gap-1.5">
                      <Spinner size={12} />
                      {downloadProgress?.percent != null
                        ? `${Math.round(downloadProgress.percent)}%`
                        : t("ダウンロード中...")}
                    </span>
                  ) : (
                    t("インストールして再起動")
                  )}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={isInstalling}
                  onClick={() => {
                    dismiss();
                  }}
                >
                  {t("閉じる")}
                </Button>
              </div>
              {isInstalling && downloadProgress && (
                <div className="mt-2 flex items-center gap-3">
                  <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-line">
                    <div
                      className="h-full bg-brand transition-all duration-150"
                      style={{
                        width: `${downloadProgress.percent ?? 0}%`,
                      }}
                    />
                  </div>
                  {downloadProgress.total_bytes && (
                    <span className="text-[10px] text-muted font-mono">
                      {(downloadProgress.downloaded_bytes / 1024 / 1024).toFixed(1)} /{" "}
                      {(downloadProgress.total_bytes / 1024 / 1024).toFixed(1)} MB
                    </span>
                  )}
                </div>
              )}
            </div>
          )}
          {installError && (
            <div className="shrink-0 border-b border-line px-3 py-2">
              <ErrorBox error={installError} />
            </div>
          )}
          {/* The menu / shortcut path goes through `createThread` directly, so
              a failure there never reached the sidebar's own error surface. */}
          {createThread.isError && (
            <div className="shrink-0 border-b border-line px-3 py-2">
              <ErrorBox
                error={
                  createThread.error || t("スレッドを作成できませんでした")
                }
              />
            </div>
          )}

          <div className="flex min-h-0 flex-1 flex-row">
            {view === "chat" && agentSurface && fileTreeOpen && (
              <Suspense fallback={<PaneFallback />}>
                <FileTreePane cap={panelCap} />
              </Suspense>
            )}
            <div className="flex min-h-0 min-w-0 flex-1 flex-col">
              {view === "chat" && <ChatPane />}
              {view !== "chat" && (
                <Suspense fallback={<PaneFallback />}>
                  {view === "agents" && <AgentsPane />}
                  {view === "projects" && <ProjectsPane />}
                  {view === "schedules" && <SchedulesPane />}
                  {view === "settings" && (
                    <PaneErrorBoundary name="SettingsPane" resetKeys={[view]}>
                      <SettingsPane />
                    </PaneErrorBoundary>
                  )}
                </Suspense>
              )}
            </div>
          </div>
        </section>
        {view === "chat" && agentSurface && (
          <PaneErrorBoundary name="RightPanel" resetKeys={[view, agentSurface]}>
            <RightPanel cap={panelCap} />
          </PaneErrorBoundary>
        )}
      </div>
      <CommandPalette />
      <ShortcutHelp />
    </div>
  );
}
