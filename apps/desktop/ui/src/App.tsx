import {
  Suspense,
  lazy,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  Activity,
  Check,
  Download,
  Folder,
  FolderTree,
  Gauge,
  GitCompareArrows,
  Globe,
  PanelLeft,
  PanelRight,
  Search,
  ShieldAlert,
} from "lucide-react";
import { useUi, type View } from "./lib/ui";
import { canFullView } from "./lib/panelTabs";
import { cn } from "./lib/cn";
import { t, tf } from "./lib/i18n";
import { chord, detectPlatformFromUa } from "./lib/platform";
import { normalizeTheme, storedTheme } from "./lib/theme";
import {
  useConversation,
  useGitStatus,
  useProjects,
  usePendingApprovals,
} from "./lib/queries";
import { useHealth } from "./lib/health";
import { useSettingsDirtyGuard, useSettingsForm } from "./lib/settings";
import {
  useUpdateBootstrap,
  useUpdateCheck,
  useUpdateStore,
} from "./lib/updates";
import { markOnce } from "./lib/perf";

// The chat pane itself is split as well: the shell (sidebar, title bar,
// command palette) paints before the transcript/composer bundle is parsed, and
// the main chunk stays under the bundle budget in `vite.config.ts`.
const ChatPane = lazy(() =>
  import("./features/chat/ChatPane").then((m) => ({ default: m.ChatPane })),
);
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
const ApprovalDrawer = lazy(() =>
  import("./features/tools/ApprovalDrawer").then((m) => ({
    default: m.ApprovalDrawer,
  })),
);
import { Sidebar } from "./features/shell/Sidebar";
import { RightPanel } from "./features/shell/RightPanel";
const CommandPalette = lazy(() =>
  import("./features/shell/CommandPalette").then((m) => ({
    default: m.CommandPalette,
  })),
);
import { HealthBanner } from "./features/shell/HealthBanner";
import { ActiveRunsIndicator } from "./features/shell/ActiveRunsIndicator";
import { QuitGuard } from "./features/shell/QuitGuard";
import { RecoveryBanner } from "./features/shell/RecoveryBanner";
const ShortcutHelp = lazy(() =>
  import("./features/shell/ShortcutHelp").then((m) => ({
    default: m.ShortcutHelp,
  })),
);
import { SurfaceSwitcher } from "./features/shell/SurfaceSwitcher";
import { WindowControls } from "./features/shell/WindowControls";
import { usePanelCap } from "./features/shell/usePanelCap";
const Onboarding = lazy(() =>
  import("./features/onboarding/Onboarding").then((m) => ({
    default: m.Onboarding,
  })),
);
import { useOnboardingGate } from "./features/onboarding/useOnboardingGate";
import {
  useAppShortcuts,
  useNewThread,
  useShellEvents,
} from "./features/shell/useShellEvents";
import { Button } from "./components/ui/button";
import { useConfirm } from "./components/ui/dialog";
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
  agents: "エージェント設定",
  projects: "プロジェクト",
  schedules: "スケジュール",
  settings: "設定",
};

function StatusMenu({ autoCheck }: { autoCheck: boolean }) {
  const openSettings = useUi((s) => s.openSettings);
  const health = useHealth();
  const check = useUpdateCheck(autoCheck);
  const pending = useUpdateStore((s) => s.pending);
  const phase = useUpdateStore((s) => s.phase);
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
  const updateLabel = pending
    ? tf("v{version} を適用待ち", { version: pending.version })
    : phase === "downloading"
      ? t("更新をダウンロード中…")
      : update
        ? tf("v{version} が利用可能です", { version: update.version })
        : "";
  const hasUpdate = updateLabel.length > 0;
  const summary = hasUpdate
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
          {hasUpdate && (
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
        {hasUpdate && (
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
  const approvalsOpen = useUi((s) => s.approvalsOpen);
  const setApprovalsOpen = useUi((s) => s.setApprovalsOpen);
  const approvals = usePendingApprovals();
  const count = approvals.data?.length ?? 0;
  if (!count) return null;
  const label = tf("承認待ちが{count}件あります", { count });
  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={() => setApprovalsOpen(!approvalsOpen)}
      title={label}
      aria-label={label}
      aria-pressed={approvalsOpen}
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
  const setSidePanelMode = useUi((s) => s.setSidePanelMode);
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
  const settings = useSettingsForm();
  // Keep the unsaved-settings warning alive while the Settings pane is not
  // mounted (e.g. after navigating to Chat).
  const settingsDirty = useSettingsDirtyGuard();
  const { confirm: confirmLeaveSettings, dialog: settingsLeaveDialog } =
    useConfirm();
  const previousView = useRef(view);
  useEffect(() => {
    const previous = previousView.current;
    previousView.current = view;
    if (previous !== "settings" || view === "settings" || !settingsDirty)
      return;
    void confirmLeaveSettings({
      title: t("設定の変更が保存されていません"),
      description: t(
        "変更は保持されますが、保存するまで反映されません。終了する前に設定画面で保存してください。",
      ),
      confirmLabel: t("移動する"),
      cancelLabel: t("設定に戻る"),
      tone: "danger",
    }).then((leave) => {
      if (!leave) setView("settings");
    });
  }, [confirmLeaveSettings, setView, settingsDirty, view]);
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

  // Check, download and stage updates once at the app root. The install itself
  // happens in Rust when the user quits normally; nothing here restarts the
  // app.
  useUpdateBootstrap({
    autoCheck: settings.data?.auto_check_updates === true,
    autoDownload: settings.data?.auto_download_updates === true,
  });

  const startNewThread = useNewThread();
  // The sidebar, file tree and docked right panel are `shrink-0` panes with
  // drag handles; each remembers its own width. `panelCap` only clamps them on
  // narrow windows.
  const [shellRow, setShellRow] = useState<HTMLDivElement | null>(null);
  const panelBounds = usePanelCap(shellRow);
  // The wizard replaces the shell's render, but these hooks stay mounted. Keep
  // their side effects quiet while it is up: otherwise ⌘K during onboarding
  // silently opened the palette and it appeared the moment setup finished,
  // and native menu events changed surface/panel state behind the wizard.
  const onboarding = useOnboardingGate();
  const shellEnabled = onboarding.ready && !onboarding.show;
  useEffect(() => {
    if (shellEnabled) markOnce("shell-ready");
  }, [shellEnabled]);
  useShellEvents({
    newThread: () => {
      if (shellEnabled) startNewThread();
    },
    openView: (target) => {
      if (shellEnabled) setView(target);
    },
    openSettingsTab: (tab) => {
      if (shellEnabled) openSettings(tab);
    },
  });
  useAppShortcuts(shellEnabled);

  // Mount the overlays only after they are first opened, so their code
  // (and the search/index bundle behind the palette) stays out of the initial
  // chunk.
  const paletteOpen = useUi((s) => s.paletteOpen);
  const approvalsOpen = useUi((s) => s.approvalsOpen);
  const shortcutsOpen = useUi((s) => s.shortcutsOpen);
  const activeProjectId = useUi((s) => s.activeProjectId);
  const activeConversation = useConversation(conversationId);
  const projects = useProjects();
  const activeProject =
    projects.data?.find((p) => p.id === activeProjectId) ?? null;
  const git = useGitStatus(
    surface === "desktop-agent" ? (activeProject?.rootPath ?? null) : null,
  );
  const changedFiles =
    git.data?.available === true ? git.data.entries.length : 0;
  const threadTitle = activeConversation.data?.title;
  const agentSurface = surface === "desktop-agent";
  // Full view only exists while the current surface can show the active
  // panel. Keeping the derived check here (instead of a persisted flag alone)
  // makes a stale "full" left over from another surface fall back to split
  // instead of hiding the chat behind the usage panel.
  const fullViewAvailable =
    view === "chat" && canFullView(rightPanel, agentSurface);
  const panelFull = fullViewAvailable && sidePanelMode === "full";

  useEffect(() => {
    if (sidePanelMode !== "full" || fullViewAvailable) return;
    setSidePanelMode("split");
  }, [fullViewAvailable, setSidePanelMode, sidePanelMode]);

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
      <Suspense
        fallback={
          <div className="flex h-full items-center justify-center bg-bg">
            <Spinner size={16} label={t("読み込み中…")} />
          </div>
        }
      >
        <Onboarding
          onComplete={onboarding.complete}
          onDefer={onboarding.complete}
        />
      </Suspense>
    );
  }

  // Full view morphs any right-panel tab into the centre column. The sidebar
  // stays; the tab bar stays on top, so switching tabs does not close the
  // workspace panel. The browser is the only tab that needs a native WebView,
  // but activity/usage/changes benefit from the same room to breathe.
  // `panelFull` is derived above, before the onboarding gate's early returns.

  return (
    <div className="relative flex h-full min-w-0 flex-col bg-bg">
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
            ? (threadTitle ??
              (agentSurface ? t("コーディング") : t("チャット")))
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
            aria-label={t("スレッド・メッセージ・コマンドを検索…")}
            aria-keyshortcuts="meta+k ctrl+k"
            title={`${t("コマンドパレット")} · ${chord("Mod", "K")}`}
            className={
              "flex h-7 min-w-7 items-center gap-2 rounded-lg border border-line bg-surface px-2 " +
              "text-xs text-faint shadow-xs transition-colors duration-100 hover:border-line-strong hover:text-muted " +
              "md:min-w-44 md:px-2.5"
            }
          >
            <Search size={13} aria-hidden="true" />
            <span className="hidden flex-1 text-start md:inline">
              {t("検索")}
            </span>
            <Kbd className="hidden md:inline-flex">{chord("Mod", "K")}</Kbd>
          </button>

          <ActiveRunsIndicator />
          <StatusMenu autoCheck={settings.data?.auto_check_updates === true} />
          <ApprovalIndicator />

          {view === "chat" && (
            <div className="flex items-center gap-1 border-s border-line ps-1.5">
              {agentSurface && (
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
              )}
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="relative"
                    title={t("表示パネル")}
                    aria-label={t("表示パネル")}
                  >
                    <PanelRight size={16} />
                    {agentSurface && changedFiles > 0 && (
                      <span
                        aria-hidden="true"
                        className="absolute -top-0.5 -right-0.5 grid min-w-3.5 place-items-center rounded-full bg-brand px-0.5 font-mono text-[9px] leading-3.5 text-on-solid"
                      >
                        {changedFiles > 99 ? "99+" : changedFiles}
                      </span>
                    )}
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="min-w-52">
                  <DropdownMenuLabel>{t("表示パネル")}</DropdownMenuLabel>
                  {agentSurface && (
                    <DropdownMenuItem
                      onSelect={() =>
                        setRightPanel(
                          rightPanel === "changes" ? "none" : "changes",
                        )
                      }
                    >
                      <GitCompareArrows
                        size={13}
                        className="text-muted"
                        aria-hidden="true"
                      />
                      <span className="flex-1">{t("変更")}</span>
                      {changedFiles > 0 && (
                        <span className="font-mono text-2xs text-faint">
                          {changedFiles > 99 ? "99+" : changedFiles}
                        </span>
                      )}
                      {rightPanel === "changes" && (
                        <Check
                          size={13}
                          className="text-brand"
                          aria-hidden="true"
                        />
                      )}
                    </DropdownMenuItem>
                  )}
                  <DropdownMenuItem
                    onSelect={() =>
                      setRightPanel(rightPanel === "usage" ? "none" : "usage")
                    }
                  >
                    <Gauge
                      size={13}
                      className="text-muted"
                      aria-hidden="true"
                    />
                    <span className="flex-1">{t("使用状況")}</span>
                    {rightPanel === "usage" && (
                      <Check
                        size={13}
                        className="text-brand"
                        aria-hidden="true"
                      />
                    )}
                  </DropdownMenuItem>
                  {agentSurface && (
                    <DropdownMenuItem
                      onSelect={() =>
                        rightPanel === "browser"
                          ? dismissBrowserPanel()
                          : openBrowserPanel("user")
                      }
                    >
                      <Globe
                        size={13}
                        className="text-muted"
                        aria-hidden="true"
                      />
                      <span className="flex-1">{t("ブラウザ")}</span>
                      {rightPanel === "browser" && (
                        <Check
                          size={13}
                          className="text-brand"
                          aria-hidden="true"
                        />
                      )}
                    </DropdownMenuItem>
                  )}
                  <DropdownMenuItem
                    onSelect={() =>
                      setRightPanel(
                        rightPanel === "activity" ? "none" : "activity",
                      )
                    }
                  >
                    <Activity
                      size={13}
                      className="text-muted"
                      aria-hidden="true"
                    />
                    <span className="flex-1">{t("アクティビティ")}</span>
                    {rightPanel === "activity" && (
                      <Check
                        size={13}
                        className="text-brand"
                        aria-hidden="true"
                      />
                    )}
                  </DropdownMenuItem>
                  {rightPanel !== "none" &&
                    rightPanel !== "changes" &&
                    rightPanel !== "usage" &&
                    rightPanel !== "activity" &&
                    rightPanel !== "browser" && (
                      <DropdownMenuItem onSelect={() => setRightPanel("none")}>
                        <PanelRight
                          size={13}
                          className="text-muted"
                          aria-hidden="true"
                        />
                        <span className="flex-1">
                          {t("サイドパネルを閉じる")}
                        </span>
                      </DropdownMenuItem>
                    )}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          )}
          <WindowControls />
        </div>
      </header>

      <HealthBanner />
      <RecoveryBanner />
      <QuitGuard />
      {settingsLeaveDialog}

      <div ref={setShellRow} className="relative flex min-h-0 min-w-0 flex-1">
        <Sidebar cap={panelBounds.cap} />
        <section
          id="main-content"
          tabIndex={-1}
          aria-label={
            view === "chat"
              ? (threadTitle ?? t("チャット"))
              : t(VIEW_TITLES[view])
          }
          className={cn(
            "flex min-w-0 flex-1 flex-col bg-bg outline-none",
            // In full view the right panel takes this column over. Hiding the
            // section instead of unmounting it keeps the chat (composer draft,
            // scroll position) alive across the transition.
            panelFull && "hidden",
          )}
        >
          {/* The menu / shortcut path goes through `newThread` and creates the
              conversation on first send, so there is no request to fail here. */}
          <div className="flex min-h-0 flex-1 flex-row">
            {view === "chat" && agentSurface && fileTreeOpen && (
              <Suspense fallback={<PaneFallback />}>
                <FileTreePane cap={panelBounds.cap} />
              </Suspense>
            )}
            <div className="flex min-h-0 min-w-0 flex-1 flex-col">
              {view === "chat" && (
                <div className="flex min-h-0 min-w-0 flex-1 flex-col">
                  <Suspense fallback={<PaneFallback />}>
                    <ChatPane />
                  </Suspense>
                </div>
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
        {view === "chat" && (
          <PaneErrorBoundary name="RightPanel" resetKeys={[view, agentSurface]}>
            <RightPanel
              rightMax={panelBounds.rightMax}
              rowWidth={panelBounds.rowWidth}
            />
          </PaneErrorBoundary>
        )}
      </div>
      {approvalsOpen && (
        <Suspense fallback={null}>
          <ApprovalDrawer />
        </Suspense>
      )}
      {paletteOpen && (
        <Suspense fallback={null}>
          <CommandPalette />
        </Suspense>
      )}
      {shortcutsOpen && (
        <Suspense fallback={null}>
          <ShortcutHelp />
        </Suspense>
      )}
    </div>
  );
}
