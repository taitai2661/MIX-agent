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
  Download,
  Folder,
  FolderTree,
  GitCompareArrows,
  Globe,
  PanelLeft,
  PanelRight,
  Search,
  ShieldAlert,
} from "lucide-react";
import { api } from "./lib/api";
import { useUi, type View } from "./lib/ui";
import { t, tf } from "./lib/i18n";
import { chord, detectPlatformFromUa } from "./lib/platform";
import { normalizeTheme, storedTheme } from "./lib/theme";
import {
  useConversations,
  useGitStatus,
  useProjects,
  usePendingApprovals,
} from "./lib/queries";
import { useHealth } from "./lib/health";
import { SETTINGS_QUERY_KEY } from "./lib/settings";
import { useUpdateCheck } from "./lib/updates";

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
const BrowserPanel = lazy(() =>
  import("./features/browser/BrowserPanel").then((m) => ({
    default: m.BrowserPanel,
  })),
);
import { Sidebar } from "./features/shell/Sidebar";
import { RightPanel } from "./features/shell/RightPanel";
import { CommandPalette } from "./features/shell/CommandPalette";
import { HealthBanner } from "./features/shell/HealthBanner";
import { ShortcutHelp } from "./features/shell/ShortcutHelp";
import { SurfaceSwitcher } from "./features/shell/SurfaceSwitcher";
import { WindowControls } from "./features/shell/WindowControls";
import { usePanelCap } from "./features/shell/usePanelCap";
import { Onboarding } from "./features/onboarding/Onboarding";
import { useOnboardingGate } from "./features/onboarding/useOnboardingGate";
import {
  useAppShortcuts,
  useNewThread,
  useShellEvents,
} from "./features/shell/useShellEvents";
import { Button } from "./components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./components/ui/dropdown-menu";
import { Kbd } from "./components/layout";
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
  agents: "エージェントプロファイル",
  projects: "プロジェクト",
  schedules: "スケジュール",
  settings: "設定",
};

function StatusMenu({ autoCheck }: { autoCheck: boolean }) {
  const openSettings = useUi((s) => s.openSettings);
  const health = useHealth();
  const check = useUpdateCheck(autoCheck);
  const update = check.data;
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
  const updateLabel = update
    ? tf("v{version} が利用可能です", { version: update.version })
    : "";
  const summary = updateLabel
    ? `${t("ヘルス")}: ${label} · ${updateLabel}`
    : `${t("ヘルス")}: ${label}`;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          className="relative"
          title={summary}
          aria-label={summary}
        >
          <StatusDot
            tone={
              state === "ok"
                ? "success"
                : state === "error"
                  ? "danger"
                  : "warning"
            }
            label={`${t("ヘルス")}: ${label}`}
            className="px-1"
          />
          {update && (
            <span
              aria-hidden="true"
              className="absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-brand"
            />
          )}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-56">
        <DropdownMenuLabel>{t("ステータス")}</DropdownMenuLabel>
        <div className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-fg">
          <StatusDot
            tone={
              state === "ok"
                ? "success"
                : state === "error"
                  ? "danger"
                  : "warning"
            }
            label=""
          />
          <span className="flex-1">
            {t("ヘルス")}: {label}
          </span>
        </div>
        <DropdownMenuItem onSelect={() => openSettings("diagnostics")}>
          <ShieldAlert size={13} className="text-muted" aria-hidden="true" />
          <span className="flex-1">{t("診断を開く")}</span>
        </DropdownMenuItem>
        {update && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => openSettings("updates")}>
              <Download size={13} className="text-brand" aria-hidden="true" />
              <span className="min-w-0 flex-1 truncate">{updateLabel}</span>
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Header chip that makes a blocked run visible from any view. */
function ApprovalIndicator() {
  const openSettings = useUi((s) => s.openSettings);
  const approvals = usePendingApprovals();
  const count = approvals.data?.length ?? 0;
  if (!count) return null;
  const label = tf("承認待ちが{count}件あります", { count });
  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={() => openSettings("approvals")}
      title={label}
      aria-label={label}
      className="gap-1 border border-warning/40 bg-warning-soft text-warning hover:bg-warning/20"
    >
      <ShieldAlert size={14} aria-hidden="true" />
      <span className="tabular-nums">{count}</span>
      <span className="hidden sm:inline">{t("承認待ち")}</span>
    </Button>
  );
}

export function App() {
  const view = useUi((s) => s.view);
  const setView = useUi((s) => s.setView);
  const sidebarOpen = useUi((s) => s.sidebarOpen);
  const toggleSidebar = useUi((s) => s.toggleSidebar);
  const rightPanel = useUi((s) => s.rightPanel);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const openBrowserPanel = useUi((s) => s.openBrowserPanel);
  const dismissBrowserPanel = useUi((s) => s.dismissBrowserPanel);
  const sidePanelMode = useUi((s) => s.sidePanelMode);
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
    document.documentElement.dataset.platform = detectPlatformFromUa(
      navigator.userAgent,
    );
    (async () => {
      try {
        const { platform } = await import("@tauri-apps/plugin-os");
        const os = await platform();
        if (!cancelled) {
          document.documentElement.dataset.platform =
            os === "macos" || os === "windows" || os === "linux" ? os : "other";
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

  const startNewThread = useNewThread();
  // The sidebar and the file tree are fixed-width `shrink-0` panes; the right
  // panel is an overlay drawer, so opening it never takes width from the
  // centre column. `panelCap` only clamps the inline panes on narrow windows.
  const [shellRow, setShellRow] = useState<HTMLDivElement | null>(null);
  const panelCap = usePanelCap(shellRow);
  useShellEvents({
    newThread: startNewThread,
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
  const git = useGitStatus(
    surface === "desktop-agent" ? (activeProject?.rootPath ?? null) : null,
  );
  const changedFiles =
    git.data?.available === true ? git.data.entries.length : 0;
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
  // Codex-style full view: the browser replaces the chat column while the
  // left sidebar stays. Splitting back is one click or ⌘⇧B.
  const browserFull =
    view === "chat" &&
    agentSurface &&
    rightPanel === "browser" &&
    sidePanelMode === "full";

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
          title={`${t("サイドバーの表示切り替え")} · ${chord("Mod", "B")}`}
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
          {view === "chat" &&
            agentSurface &&
            (activeProject ? (
              <button
                type="button"
                onClick={() => setView("projects")}
                className="ms-2 hidden max-w-48 truncate align-middle text-xs font-normal text-faint hover:text-fg hover:bg-hover px-1.5 py-0.5 rounded transition-colors sm:inline-flex items-center gap-1"
                title={`${t("プロジェクトを変更")}: ${activeProject.rootPath ?? activeProject.name}`}
              >
                <span className="text-line">·</span>
                <Folder size={11} className="text-muted shrink-0" />
                <span className="truncate">{activeProject.name}</span>
              </button>
            ) : (
              <button
                type="button"
                onClick={() => setView("projects")}
                className="ms-2 hidden truncate align-middle text-2xs font-normal text-warning hover:text-warning/80 hover:bg-warning-soft px-1.5 py-0.5 rounded border border-warning/30 transition-colors sm:inline-flex items-center gap-1"
                title={t("プロジェクトを選択")}
              >
                <span className="text-line">·</span>
                <span>{t("プロジェクト未選択")}</span>
              </button>
            ))}
        </h1>

        <div className="flex shrink-0 items-center gap-1.5">
          <button
            type="button"
            onClick={() => setPaletteOpen(true)}
            aria-label={t("コマンド・スレッドを検索…")}
            aria-keyshortcuts="meta+k ctrl+k"
            title={`${t("コマンドパレット")} · ${chord("Mod", "K")}`}
            className={
              "hidden h-7 min-w-44 items-center gap-2 rounded-lg border border-line bg-surface px-2.5 " +
              "text-xs text-faint shadow-xs transition-colors duration-100 hover:border-line-strong hover:text-muted md:flex"
            }
          >
            <Search size={13} aria-hidden="true" />
            <span className="flex-1 text-start">{t("検索")}</span>
            <Kbd>{chord("Mod", "K")}</Kbd>
          </button>

          <StatusMenu autoCheck={settings.data?.auto_check_updates === true} />
          <ApprovalIndicator />

          {view === "chat" && agentSurface && (
            <div className="flex items-center gap-1 border-s border-line ps-1.5">
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={toggleFileTree}
                title={`${t("ファイルツリー")} · ${chord("Mod", "Shift", "E")}`}
                aria-label={t("ファイルツリー")}
                aria-pressed={fileTreeOpen}
              >
                <FolderTree size={16} />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() =>
                  setRightPanel(rightPanel === "changes" ? "none" : "changes")
                }
                title={
                  changedFiles > 0
                    ? tf("変更を確認 ({count})", { count: changedFiles })
                    : t("変更を確認")
                }
                aria-label={
                  changedFiles > 0
                    ? tf("変更を確認 ({count}件)", { count: changedFiles })
                    : t("変更を確認")
                }
                aria-pressed={rightPanel === "changes"}
                className="relative"
              >
                <GitCompareArrows size={16} />
                {changedFiles > 0 && (
                  <span
                    aria-hidden="true"
                    className="absolute -top-0.5 -right-0.5 grid min-w-3.5 place-items-center rounded-full bg-brand px-0.5 font-mono text-[9px] leading-3.5 text-on-solid"
                  >
                    {changedFiles > 99 ? "99+" : changedFiles}
                  </span>
                )}
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() =>
                  rightPanel === "browser"
                    ? dismissBrowserPanel()
                    : openBrowserPanel("user")
                }
                title={
                  rightPanel === "browser"
                    ? t("ブラウザを閉じる")
                    : t("ブラウザを表示")
                }
                aria-label={
                  rightPanel === "browser"
                    ? t("ブラウザを閉じる")
                    : t("ブラウザを表示")
                }
                aria-pressed={rightPanel === "browser"}
              >
                <Globe size={16} />
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

      <HealthBanner />

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
          {/* The menu / shortcut path goes through `newThread` and creates the
              conversation on first send, so there is no request to fail here. */}
          <div className="flex min-h-0 flex-1 flex-row">
            {view === "chat" &&
              agentSurface &&
              fileTreeOpen &&
              !browserFull && (
                <Suspense fallback={<PaneFallback />}>
                  <FileTreePane cap={panelCap} />
                </Suspense>
              )}
            <div className="flex min-h-0 min-w-0 flex-1 flex-col">
              {/* Keep the chat mounted in full browser view: unmounting it
                  discarded the composer draft and scroll position. */}
              {view === "chat" && (
                <div
                  className={
                    browserFull
                      ? "hidden"
                      : "flex min-h-0 min-w-0 flex-1 flex-col"
                  }
                >
                  <ChatPane />
                </div>
              )}
              {browserFull && (
                <Suspense fallback={<PaneFallback />}>
                  <PaneErrorBoundary name="BrowserPanel" resetKeys={[view]}>
                    <div className="flex min-h-0 flex-1 flex-col">
                      <BrowserPanel />
                    </div>
                  </PaneErrorBoundary>
                </Suspense>
              )}
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
        {view === "chat" && agentSurface && !browserFull && (
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
