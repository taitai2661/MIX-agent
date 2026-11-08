import { BrowserInstallNotifier } from "@/app/browser-install-notifier";
import { api, setCSRF, type Row } from "@/app/api";
import { getLanguage, setLanguage, t, useLanguage } from "@/app/i18n";
import { ja } from "@/app/strings";
import { AppSidebar } from "@/app/AppSidebar";
import {
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
} from "@/app/sidebarConfig";
import { Button } from "@/components/button";
import { Empty, ErrorBox } from "@/components/shared";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { Tour } from "@/components/tour";
import { Agents } from "@/features/agents/Agents";
import { Auth } from "@/features/auth/Auth";
import { Conversations } from "@/features/chat/Conversations";
import type { WorkspaceSnapshot } from "@/features/chat/greeting";
import { Projects } from "@/features/projects/Projects";
import { Schedules } from "@/features/schedules/Schedules";
import { Settings } from "@/features/settings/Settings";
import { Setup } from "@/features/setup/Setup";
import { Skills } from "@/features/skills/Skills";
import {
  CONVERSATION_PAGE_SIZE,
  nextConversationOffset,
} from "@/lib/conversations";
import { queryKeys } from "@/lib/queries";
import { safeStorage } from "@/lib/storage";
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { PanelLeft, PanelLeftOpen, Settings2, X } from "lucide-react";
import { Suspense, useEffect, useMemo, useRef, useState } from "react";
import {
  Link,
  Navigate,
  Route,
  Routes,
  useLocation,
  useNavigate,
  useParams,
} from "react-router-dom";
import { Chat } from "@/features/chat/Chat";

function useMediaQuery(query: string) {
  const [matches, setMatches] = useState(
    () => typeof window !== "undefined" && window.matchMedia(query).matches,
  );
  useEffect(() => {
    const list = window.matchMedia(query);
    const update = () => setMatches(list.matches);
    update();
    list.addEventListener("change", update);
    return () => list.removeEventListener("change", update);
  }, [query]);
  return matches;
}

type Me = { username: string; csrf: string };
type SetupInfo = { needs_admin: boolean };
type SettingsInfo = {
  data: { ui_language?: string; setup_complete?: boolean };
};

/** Keyed wrapper so switching conversations remounts the composer. That lets
 *  the per-conversation draft survive a session-expiry round trip through the
 *  login screen instead of vanishing with the unmounted component. */
function ChatRoute({ workspace }: { workspace: WorkspaceSnapshot | null }) {
  const { id } = useParams();
  return <Chat key={id ?? "new"} workspace={workspace} />;
}

/** Project chat must remount when the project changes, mirroring ChatRoute. */
function ProjectChatRoute({
  workspace,
}: {
  workspace: WorkspaceSnapshot | null;
}) {
  const { projectId } = useParams();
  return <Chat key={projectId ?? "new"} workspace={workspace} />;
}

export function App() {
  useLanguage();
  const [user, setUser] = useState<Me | null>(null),
    [ready, setReady] = useState(false),
    [needsAdmin, setNeedsAdmin] = useState(false),
    [setupComplete, setSetupComplete] = useState<boolean | null>(null),
    [setupBannerDismissed, setSetupBannerDismissed] = useState(false),
    [error, setError] = useState<unknown>(null),
    [sessionExpired, setSessionExpired] = useState(false),
    [sidebar, setSidebar] = useState(false),
    [collapsed, setCollapsed] = useState(
      () => safeStorage.get("mix-agent-sidebar-collapsed") === "1",
    ),
    [resizingSidebar, setResizingSidebar] = useState(false),
    [sidebarWidth, setSidebarWidth] = useState(() => {
      const stored = Number(safeStorage.get("mix-agent-sidebar-width"));
      return Number.isFinite(stored) &&
        stored >= SIDEBAR_MIN_WIDTH &&
        stored <= SIDEBAR_MAX_WIDTH
        ? stored
        : SIDEBAR_DEFAULT_WIDTH;
    });
  const isMobile = useMediaQuery("(max-width: 760px)");
  const navigate = useNavigate();
  const location = useLocation();
  const firstRoute = useRef(true);
  const previousPath = useRef(location.pathname);
  const sidebarToggleRef = useRef<HTMLButtonElement>(null);
  const qc = useQueryClient();
  const [showTour, setShowTour] = useState(false);
  useEffect(() => {
    // iOS Safari keeps `100dvh` at the pre-keyboard height. Publish the visual
    // viewport so the mobile shell and fixed bottom sheets stay above the
    // keyboard. Desktop has no visualViewport resize, so these values stay at
    // the viewport size and the CSS fallbacks remain in charge.
    const viewport = window.visualViewport;
    if (!viewport) return;
    const root = document.documentElement;
    const update = () => {
      const height = Math.round(viewport.height);
      const keyboardInset = Math.max(
        0,
        Math.round(window.innerHeight - viewport.height - viewport.offsetTop),
      );
      root.style.setProperty("--visual-viewport-height", `${height}px`);
      root.style.setProperty(
        "--visual-viewport-bottom-inset",
        `${keyboardInset}px`,
      );
    };
    update();
    viewport.addEventListener("resize", update);
    viewport.addEventListener("scroll", update);
    window.addEventListener("orientationchange", update);
    return () => {
      viewport.removeEventListener("resize", update);
      viewport.removeEventListener("scroll", update);
      window.removeEventListener("orientationchange", update);
      root.style.removeProperty("--visual-viewport-height");
      root.style.removeProperty("--visual-viewport-bottom-inset");
    };
  }, []);
  useEffect(() => {
    // Returning from setup must refresh `setup_complete`; otherwise the banner
    // either never appears after "あとで設定する" or never disappears after
    // the final step.
    const wasSetup = previousPath.current.startsWith("/setup");
    const isSetup = location.pathname.startsWith("/setup");
    previousPath.current = location.pathname;
    if (!user || !wasSetup || isSetup) return;
    // Reset only when returning from the wizard. Doing it on every navigation
    // made a dismissed banner reappear as soon as the user opened any page.
    setSetupBannerDismissed(false);
    let cancelled = false;
    api<{ data: { ui_language?: string; setup_complete?: boolean } }>(
      "/settings",
    )
      .then((settings) => {
        if (cancelled) return;
        setSetupComplete(settings.data.setup_complete === false ? false : true);
        setLanguage(settings.data.ui_language === "en" ? "en" : "ja");
      })
      .catch(() => {
        // Keep the previous banner state; the page itself still works.
      });
    void qc.invalidateQueries({ queryKey: queryKeys.settings });
    return () => {
      cancelled = true;
    };
  }, [location.pathname, qc, user]);
  useEffect(() => {
    Promise.all([
      api<SetupInfo>("/setup"),
      api<Me>("/auth/me").catch(() => null),
    ])
      .then(async ([setup, me]) => {
        setNeedsAdmin(setup.needs_admin);
        if (me) {
          setCSRF(me.csrf);
          const settings = await api<SettingsInfo>("/settings");
          setLanguage(settings.data.ui_language === "en" ? "en" : "ja");
          setSetupComplete(
            settings.data.setup_complete === false ? false : true,
          );
          setUser(me);
        }
        setReady(true);
        // 初回ログイン時にツアーを表示
        if (me && !safeStorage.get("mix-agent-tour-done")) {
          setShowTour(true);
        }
      })
      .catch(setError);
  }, []);
  const conversations = useInfiniteQuery({
    queryKey: ["/conversations", "sidebar"],
    initialPageParam: 0,
    queryFn: ({ pageParam, signal }) =>
      api<Row[]>(
        `/conversations?state=active&limit=${CONVERSATION_PAGE_SIZE}&offset=${pageParam}`,
        "GET",
        undefined,
        undefined,
        { signal },
      ),
    getNextPageParam: (lastPage, _pages, lastPageParam) =>
      nextConversationOffset(lastPage.length, lastPageParam),
    enabled: !!user,
  });
  // The sidebar and the greeting both read this flattened view; the cache is
  // still one paginated query, so "load older" adds to the same list.
  const conversationRows = useMemo(
    () => conversations.data?.pages.flatMap((page) => page) ?? [],
    [conversations.data],
  );
  useEffect(() => {
    // A session can expire while the tab is open.  The API client emits this
    // event instead of surfacing a generic network error, so drop the cached
    // identity and let the Auth screen take over without a full reload.
    const unauthorized = () => {
      setCSRF("");
      setSessionExpired(true);
      setUser(null);
      // Dropping the identity is not enough: React Query still holds the
      // previous account's conversations, so a shared browser would show them
      // for a moment after the next login. Clearing the cache here matches an
      // explicit logout.
      qc.clear();
    };
    window.addEventListener("mix:unauthorized", unauthorized);
    return () => window.removeEventListener("mix:unauthorized", unauthorized);
  }, [qc]);
  useEffect(() => {
    const path = location.pathname;
    const label =
      path === "/"
        ? "新しいチャット"
        : path.startsWith("/chat/")
          ? "チャット"
          : path.startsWith("/projects")
            ? "プロジェクト"
            : path.startsWith("/history")
              ? "会話"
              : path.startsWith("/agents")
                ? "エージェント"
                : path.startsWith("/skills")
                  ? "スキル"
                  : path.startsWith("/schedules")
                    ? "定期実行"
                    : path.startsWith("/settings")
                      ? "設定"
                      : path.startsWith("/setup")
                        ? "セットアップ"
                        : "MIX agent";
    // A conversation owns its own title (set by Chat from the last turn).
    if (!path.startsWith("/chat/")) {
      document.title =
        label === "MIX agent" ? label : `${t(label)} · MIX agent`;
    }
    if (firstRoute.current) {
      firstRoute.current = false;
      return;
    }
    // Announce the new page to keyboard/screen-reader users. Every route had
    // the same document title and left focus wherever the previous page was.
    document.getElementById("main-content")?.focus({ preventScroll: true });
  }, [location.pathname]);
  // 新しいチャット画面の挨拶に使う、ワークスペースの状況だけを渡す。
  const workspace = useMemo<WorkspaceSnapshot | null>(() => {
    if (!conversations.data) return null;
    const rows = conversationRows;
    const stamps = rows
      .map((row) =>
        Date.parse(String(row.data.last_message_at || row.created_at)),
      )
      .filter((stamp) => Number.isFinite(stamp));
    return {
      conversationCount: rows.length,
      pinnedCount: rows.filter((row) => row.data.pinned).length,
      projectCount: 0,
      agentCount: 0,
      lastMessageAt: stamps.length ? Math.max(...stamps) : null,
    };
  }, [conversationRows, conversations.data]);
  async function authenticated(value: Me) {
    setCSRF(value.csrf);
    setSessionExpired(false);
    setUser(value);
    try {
      if (needsAdmin)
        await api("/settings", "PUT", { ui_language: getLanguage() });
      const settings = await api<SettingsInfo>("/settings");
      setLanguage(settings.data.ui_language === "en" ? "en" : "ja");
      setSetupComplete(settings.data.setup_complete === false ? false : true);
    } catch (cause) {
      setError(cause);
      return;
    }
    if (needsAdmin) void navigate("/setup");
    setNeedsAdmin(false);
  }
  if (error)
    return (
      <main className="auth">
        <ErrorBox error={error} />
        <Button onClick={() => window.location.reload()}>
          {t("再読み込み")}
        </Button>
      </main>
    );
  if (!ready) return <main className="auth">{t(ja.loading)}</main>;
  if (!user)
    return (
      <Auth
        setup={needsAdmin}
        sessionExpired={sessionExpired}
        onDone={authenticated}
      />
    );
  return (
    <>
      <BrowserInstallNotifier />
      <div
        className={
          "shell " +
          (sidebar ? "sidebar-open " : "") +
          (collapsed ? "sidebar-collapsed " : "") +
          (resizingSidebar ? "sidebar-resizing" : "")
        }
      >
        <a className="skip-link" href="#main-content">
          {t("本文へスキップ")}
        </a>

        <AppSidebar
          user={user}
          rows={conversationRows}
          hasMoreConversations={Boolean(conversations.hasNextPage)}
          loadingMoreConversations={conversations.isFetchingNextPage}
          onLoadMoreConversations={() => void conversations.fetchNextPage()}
          isMobile={isMobile}
          open={sidebar}
          onOpenChange={setSidebar}
          collapsed={collapsed}
          onCollapsedChange={setCollapsed}
          width={sidebarWidth}
          onWidthChange={setSidebarWidth}
          onResizingChange={setResizingSidebar}
          toggleRef={sidebarToggleRef}
          onLoggedOut={() => setUser(null)}
        />
        <main id="main-content" className="main" tabIndex={-1}>
          <header className="topbar">
            {isMobile || collapsed ? (
              <>
                <button
                  className="icon"
                  ref={sidebarToggleRef}
                  aria-label={
                    isMobile
                      ? sidebar
                        ? t("メニューを閉じる")
                        : t("メニュー")
                      : t("サイドバーを展開する")
                  }
                  aria-expanded={isMobile ? sidebar : undefined}
                  aria-controls="main-navigation"
                  onClick={() =>
                    isMobile ? setSidebar(!sidebar) : setCollapsed(!collapsed)
                  }
                >
                  {isMobile ? (
                    sidebar ? (
                      <X size={19} />
                    ) : (
                      <PanelLeft size={19} />
                    )
                  ) : (
                    <PanelLeftOpen size={19} />
                  )}
                </button>
                <span className="topbar-title">
                  <b>MIX agent</b>
                </span>
              </>
            ) : null}
          </header>
          {setupComplete === false &&
            !setupBannerDismissed &&
            !location.pathname.startsWith("/setup") && (
              <div className="setup-banner" role="status">
                <span className="setup-banner-icon">
                  <Settings2 size={16} />
                </span>
                <p>
                  <b>{t("セットアップが完了していません")}</b>
                  <span>
                    {t(
                      "AI Providerの接続など、残りの設定を完了するとチャットを始められます。",
                    )}
                  </span>
                </p>
                <Link className="button btn-primary" to="/setup">
                  {t("セットアップを続ける")}
                </Link>
                <button
                  type="button"
                  className="icon setup-banner-dismiss"
                  aria-label={t("閉じる")}
                  onClick={() => setSetupBannerDismissed(true)}
                >
                  <X size={16} />
                </button>
              </div>
            )}
          <ErrorBoundary>
            <Suspense
              fallback={<div className="page-loading">{t("読み込み中…")}</div>}
            >
              <Routes>
                <Route path="/" element={<ChatRoute workspace={workspace} />} />
                <Route
                  path="/chat/:id"
                  element={<ChatRoute workspace={workspace} />}
                />
                <Route
                  path="/projects/:projectId/chat"
                  element={<ProjectChatRoute workspace={workspace} />}
                />
                <Route path="/projects" element={<Projects />} />
                <Route path="/projects/:id" element={<Projects />} />
                <Route path="/history" element={<Conversations />} />
                <Route path="/history/:state" element={<Conversations />} />
                <Route path="/agents" element={<Agents />} />
                <Route
                  path="/memory"
                  element={<Navigate to="/settings/memory" replace />}
                />
                <Route path="/skills" element={<Skills />} />
                <Route path="/schedules" element={<Schedules />} />
                <Route path="/setup" element={<Setup />} />
                <Route
                  path="/settings/*"
                  element={
                    <Settings
                      user={user}
                      onUserChange={(value) => {
                        setCSRF(value.csrf);
                        setUser(value);
                      }}
                      onLogout={() => {
                        setUser(null);
                        qc.clear();
                      }}
                    />
                  }
                />
                <Route
                  path="*"
                  element={<Empty>{t("ページが見つかりません")}</Empty>}
                />
              </Routes>
            </Suspense>
          </ErrorBoundary>
          {showTour && (
            <Tour
              onClose={() => {
                setShowTour(false);
                safeStorage.set("mix-agent-tour-done", "1");
              }}
            />
          )}
        </main>
      </div>
    </>
  );
}
